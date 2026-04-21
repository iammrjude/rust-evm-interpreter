use primitive_types::U256;
use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct Storage {
    slots: HashMap<U256, U256>,
}

impl Storage {
    pub fn new() -> Self {
        Self {
            slots: HashMap::new(),
        }
    }

    pub fn load(&self, key: U256) -> U256 {
        self.slots.get(&key).copied().unwrap_or_else(U256::zero)
    }

    pub fn store(&mut self, key: U256, value: U256) {
        self.slots.insert(key, value);
    }

    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }
}
