use primitive_types::U256;

#[derive(Debug, Clone, Default)]
pub struct Memory {
    data: Vec<u8>,
}

impl Memory {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    fn ensure_size(&mut self, end: usize) {
        if end > self.data.len() {
            self.data.resize(end, 0);
        }
    }

    pub fn read_slice(&mut self, offset: usize, size: usize) -> Vec<u8> {
        if size == 0 {
            return Vec::new();
        }
        let end = offset.saturating_add(size);
        self.ensure_size(end);
        self.data[offset..end].to_vec()
    }

    pub fn write_slice(&mut self, offset: usize, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let end = offset.saturating_add(bytes.len());
        self.ensure_size(end);
        self.data[offset..end].copy_from_slice(bytes);
    }

    pub fn mload(&mut self, offset: usize) -> U256 {
        let bytes = self.read_slice(offset, 32);
        U256::from_big_endian(&bytes)
    }

    pub fn mstore(&mut self, offset: usize, value: U256) {
        let bytes = value.to_big_endian();
        self.write_slice(offset, &bytes);
    }

    pub fn mstore8(&mut self, offset: usize, value: U256) {
        let byte = (value.low_u32() & 0xff) as u8;
        self.write_slice(offset, &[byte]);
    }

    pub fn mcopy(&mut self, dst: usize, src: usize, len: usize) {
        if len == 0 {
            return;
        }
        let copied = self.read_slice(src, len);
        self.write_slice(dst, &copied);
    }
}
