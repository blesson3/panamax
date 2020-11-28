use iron::prelude::*;
use iron::AfterMiddleware;

pub struct CorsMiddleware;

impl AfterMiddleware for CorsMiddleware {
    fn after(&self, _req: &mut Request, mut res: Response) -> IronResult<Response> {
        res.headers
            .set(iron::headers::AccessControlAllowOrigin::Any);
        Ok(res)
    }
}
