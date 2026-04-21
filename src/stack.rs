use primitive_types::U256;

pub const STACK_LIMIT: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackError {
    Overflow,
    Underflow,
    InvalidDepth,
}

#[derive(Debug, Clone, Default)]
pub struct Stack {
    data: Vec<U256>,
}

impl Stack {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn push(&mut self, value: U256) -> Result<(), StackError> {
        if self.data.len() >= STACK_LIMIT {
            return Err(StackError::Overflow);
        }
        self.data.push(value);
        Ok(())
    }

    pub fn pop(&mut self) -> Result<U256, StackError> {
        self.data.pop().ok_or(StackError::Underflow)
    }

    pub fn peek(&self, depth_from_top: usize) -> Result<U256, StackError> {
        if depth_from_top >= self.data.len() {
            return Err(StackError::Underflow);
        }
        let idx = self.data.len() - 1 - depth_from_top;
        Ok(self.data[idx])
    }

    pub fn dup(&mut self, n: usize) -> Result<(), StackError> {
        if n == 0 || n > 16 {
            return Err(StackError::InvalidDepth);
        }
        let value = self.peek(n - 1)?;
        self.push(value)
    }

    pub fn swap(&mut self, n: usize) -> Result<(), StackError> {
        if n == 0 || n > 16 {
            return Err(StackError::InvalidDepth);
        }
        let len = self.data.len();
        if len <= n {
            return Err(StackError::Underflow);
        }
        let top = len - 1;
        let other = len - 1 - n;
        self.data.swap(top, other);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub fn as_slice(&self) -> &[U256] {
        &self.data
    }
}
