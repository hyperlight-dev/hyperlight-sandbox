use std::ops::{Deref, DerefMut};
use std::pin::pin;
use std::sync::Arc;

pub use hyperlight_sandbox::runtime::BlockOn;

use crate::wasi_impl::types::pollable::AnyPollable;

pub struct Resource<T> {
    inner: Arc<tokio::sync::RwLock<T>>,
    notify: Arc<tokio::sync::Notify>,
}

impl<T: Default> Default for Resource<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

impl<T> Clone for Resource<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            notify: self.notify.clone(),
        }
    }
}

impl<T> Resource<T> {
    pub fn new(val: T) -> Self {
        Self {
            inner: Arc::new(tokio::sync::RwLock::new(val)),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }

    pub fn notify(&self) {
        self.notify.notify_waiters();
    }

    pub async fn read(&self) -> ReadGuard<T> {
        ReadGuard {
            guard: self.inner.clone().read_owned().await,
        }
    }

    pub async fn write(&self) -> WriteGuard<T> {
        WriteGuard {
            guard: self.inner.clone().write_owned().await,
            resource: self.clone(),
            do_notify: true,
        }
    }

    async fn wait_impl<G: Guard<Target = T>>(&self, guard: G) -> G {
        let fut = self.notify.notified();
        let mut fut = pin!(fut);
        fut.as_mut().enable();

        G::unlock(guard);

        fut.await;
        G::lock(self.clone()).await
    }

    pub async fn read_wait_until(&self, mut cond: impl FnMut(&T) -> bool) -> ReadGuard<T> {
        let mut guard = self.read().await;
        while !cond(&*guard) {
            guard = self.wait_impl(guard).await;
        }
        guard
    }

    pub async fn write_wait_until(&self, mut cond: impl FnMut(&T) -> bool) -> WriteGuard<T> {
        let mut guard = self.write().await;
        while !cond(&*guard) {
            guard = self.wait_impl(guard).await;
        }
        guard
    }

    pub fn poll(&self, cond: impl Fn(&T) -> bool + Send + Sync + 'static) -> Resource<AnyPollable>
    where
        T: Send + Sync + 'static,
    {
        Resource::new(AnyPollable::resource(self.clone(), cond))
    }
}

pub struct ReadGuard<T> {
    guard: tokio::sync::OwnedRwLockReadGuard<T>,
}

impl<T> ReadGuard<T> {
    fn drop_no_notify(self) {
        drop(self);
    }
}

impl<T> Deref for ReadGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

pub struct WriteGuard<T> {
    guard: tokio::sync::OwnedRwLockWriteGuard<T>,
    resource: Resource<T>,
    do_notify: bool,
}

impl<T> WriteGuard<T> {
    fn drop_no_notify(mut self) {
        self.do_notify = false;
        drop(self);
    }
}

impl<T> Deref for WriteGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}
impl<T> DerefMut for WriteGuard<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}
impl<T> Drop for WriteGuard<T> {
    fn drop(&mut self) {
        if self.do_notify {
            self.resource.notify();
        }
    }
}

trait Guard: Sized {
    type Target;
    async fn lock(res: Resource<Self::Target>) -> Self;
    fn unlock(self);
}

impl<T> Guard for ReadGuard<T> {
    type Target = T;
    async fn lock(res: Resource<Self::Target>) -> Self {
        res.read().await
    }
    fn unlock(self) {
        self.drop_no_notify();
    }
}

impl<T> Guard for WriteGuard<T> {
    type Target = T;
    async fn lock(res: Resource<Self::Target>) -> Self {
        res.write().await
    }
    fn unlock(self) {
        self.drop_no_notify();
    }
}
