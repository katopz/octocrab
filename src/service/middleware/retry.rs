use http::{Request, Response};
use hyper_util::client::legacy::Error;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tower::retry::Policy;

use crate::body::OctoBody;
use crate::internal::async_runtime::sleep;

#[derive(Clone)]
pub enum RetryConfig {
    None,
    Simple(usize),
}

impl<B> Policy<Request<OctoBody>, Response<B>, Error> for RetryConfig {
    type Future = Pin<Box<dyn Future<Output = ()> + Send + Sync + 'static>>;

    fn retry(
        &mut self,
        _req: &mut Request<OctoBody>,
        result: &mut Result<Response<B>, Error>,
    ) -> Option<Self::Future> {
        match self {
            RetryConfig::None => None,
            RetryConfig::Simple(count) => match result {
                Ok(response) => {
                    if response.status().is_server_error() || response.status() == 429 {
                        if *count > 0 {
                            *count -= 1;
                            // Exponential backoff: delay doubles with each retry attempt
                            let attempt = 3 - *count;
                            let delay_ms = 2u64.pow(attempt.min(6) as u32) * 100;
                            let delay = sleep(Duration::from_millis(delay_ms));
                            Some(Box::pin(async move {
                                delay.await;
                            }))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
                Err(_) => {
                    if *count > 0 {
                        *count -= 1;
                        // Exponential backoff: delay doubles with each retry attempt
                        let attempt = 3 - *count;
                        let delay_ms = 2u64.pow(attempt.min(6) as u32) * 100;
                        let delay = sleep(Duration::from_millis(delay_ms));
                        Some(Box::pin(async move {
                            delay.await;
                        }))
                    } else {
                        None
                    }
                }
            },
        }
    }

    fn clone_request(&mut self, req: &Request<OctoBody>) -> Option<Request<OctoBody>> {
        match self {
            RetryConfig::None => None,
            _ => {
                let body = req.body().try_clone()?;

                // `Request` can't be cloned
                let mut new_req = Request::builder()
                    .uri(req.uri())
                    .method(req.method())
                    .version(req.version());
                for (name, value) in req.headers() {
                    new_req = new_req.header(name, value);
                }

                let new_req = new_req.body(body).expect(
                    "This should never panic, as we are cloning a components from existing request",
                );
                Some(new_req)
            }
        }
    }
}
