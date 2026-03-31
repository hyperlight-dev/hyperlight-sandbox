use std::pin::Pin;
use std::task::{Context, Poll};

use pin_project::pin_project;

use crate::wasi_impl::resource::Resource;

#[pin_project]
struct PollableFuture<F: Future> {
    #[pin]
    fut: Option<F>,
}

impl<F: Future> Future for PollableFuture<F> {
    type Output = bool;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<bool> {
        let mut this = self.project();
        match this.fut.as_mut().as_pin_mut() {
            None => Poll::Ready(true),
            Some(fut) => match fut.poll(cx) {
                Poll::Pending => Poll::Ready(false),
                Poll::Ready(_) => {
                    this.fut.set(None);
                    Poll::Ready(true)
                }
            },
        }
    }
}

pub struct AnyPollable {
    fut: PollableFuture<Pin<Box<dyn Future<Output = ()> + Send + Sync>>>,
}

impl AnyPollable {
    pub fn future(f: impl Future + Send + Sync + 'static) -> Self {
        let fut = async move {
            f.await;
        };
        let fut = PollableFuture {
            fut: Some(Box::pin(fut) as _),
        };
        Self { fut }
    }

    pub fn resource<T: Send + Sync + 'static>(
        res: Resource<T>,
        cond: impl Fn(&T) -> bool + Send + Sync + 'static,
    ) -> Self {
        Self::future(async move {
            res.read_wait_until(cond).await;
        })
    }

    pub async fn ready(&mut self) -> bool {
        let fut = &mut self.fut;
        let fut = Pin::new(fut);
        fut.await
    }

    pub async fn block(&mut self) {
        // Cap iterations to prevent infinite busy-wait if a pollable never becomes ready.
        const MAX_BLOCK_ITERATIONS: usize = 100_000;
        for _ in 0..MAX_BLOCK_ITERATIONS {
            if self.ready().await {
                return;
            }
            tokio::task::yield_now().await;
        }
        log::warn!("pollable::block() exceeded {MAX_BLOCK_ITERATIONS} iterations; giving up");
    }

    pub fn poll(&mut self, cx: &mut Context<'_>) -> Poll<bool> {
        let fut = &mut self.fut;
        let fut = Pin::new(fut);
        fut.poll(cx)
    }
}
