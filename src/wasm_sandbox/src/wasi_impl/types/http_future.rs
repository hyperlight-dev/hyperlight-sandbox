pub struct FutureHttp<T> {
    value: Option<T>,
    consumed: bool,
}

impl<T> Default for FutureHttp<T> {
    fn default() -> Self {
        Self {
            value: None,
            consumed: false,
        }
    }
}

impl<T> FutureHttp<T> {
    pub fn set(&mut self, value: T) {
        self.value = Some(value);
    }

    pub fn get(&mut self) -> Option<Result<T, ()>> {
        if self.consumed {
            return Some(Err(()));
        }
        if let Some(value) = self.value.take() {
            self.consumed = true;
            return Some(Ok(value));
        }
        None
    }

    pub fn is_ready(&self) -> bool {
        self.consumed || self.value.is_some()
    }
}
