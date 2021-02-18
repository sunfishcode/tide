use crate::log;
use crate::{Body, Endpoint, Request, Response, Result, StatusCode};

use async_std::io::BufReader;

use cap_async_std::fs;

pub(crate) struct ServeDir {
    prefix: String,
    dir: fs::Dir,
}

impl ServeDir {
    /// Create a new instance of `ServeDir`.
    pub(crate) fn new(prefix: String, dir: fs::Dir) -> Self {
        Self { prefix, dir }
    }
}

#[async_trait::async_trait]
impl<State> Endpoint<State> for ServeDir
where
    State: Clone + Send + Sync + 'static,
{
    async fn call(&self, req: Request<State>) -> Result {
        let path = req.url().path();
        let path = path
            .strip_prefix(&self.prefix.trim_end_matches('*'))
            .unwrap();
        let path = path.trim_start_matches('/');

        log::info!("Requested file: {:?}", path);

        let file = match self.dir.open(path).await {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                log::warn!("Unauthorized attempt to read: {:?}", path);
                return Ok(Response::new(StatusCode::Forbidden));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                log::warn!("File not found: {:?}", path);
                return Ok(Response::new(StatusCode::NotFound));
            }
            Err(e) => return Err(e.into()),
        };

        // TODO: This always uses `mime::BYTE_STREAM`; with http-types 3.0
        // we'll be able to use `Body::from_open_file` which fixes this.
        let body = Body::from_reader(BufReader::new(file), None);
        Ok(Response::builder(StatusCode::Ok).body(body).build())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use async_std::io::WriteExt;
    use cap_async_std::ambient_authority;
    use cap_async_std::fs::Dir;

    fn serve_dir(tempdir: &tempfile::TempDir) -> crate::Result<ServeDir> {
        let static_dir = async_std::task::block_on(async { setup_static_dir(tempdir).await })?;

        Ok(ServeDir {
            prefix: "/static/".to_string(),
            dir: static_dir,
        })
    }

    async fn setup_static_dir(tempdir: &tempfile::TempDir) -> crate::Result<Dir> {
        let static_dir = tempdir.path().join("static");
        Dir::create_ambient_dir_all(&static_dir, ambient_authority()).await?;

        let static_dir = Dir::open_ambient_dir(static_dir, ambient_authority()).await?;
        let mut file = static_dir.create("foo").await?;
        write!(file, "Foobar").await?;
        Ok(static_dir)
    }

    fn request(path: &str) -> crate::Request<()> {
        let request = crate::http::Request::get(
            crate::http::Url::parse(&format!("http://localhost/{}", path)).unwrap(),
        );
        crate::Request::new((), request, vec![])
    }

    #[async_std::test]
    async fn ok() {
        let tempdir = tempfile::tempdir().unwrap();
        let serve_dir = serve_dir(&tempdir).unwrap();

        let req = request("static/foo");

        let res = serve_dir.call(req).await.unwrap();
        let mut res: crate::http::Response = res.into();

        assert_eq!(res.status(), 200);
        assert_eq!(res.body_string().await.unwrap(), "Foobar");
    }

    #[async_std::test]
    async fn not_found() {
        let tempdir = tempfile::tempdir().unwrap();
        let serve_dir = serve_dir(&tempdir).unwrap();

        let req = request("static/bar");

        let res = serve_dir.call(req).await.unwrap();
        let res: crate::http::Response = res.into();

        assert_eq!(res.status(), 404);
    }
}
