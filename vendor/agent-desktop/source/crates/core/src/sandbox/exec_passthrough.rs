#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecPassthrough {
    pub stdout: Vec<u8>,
    pub status: i32,
}
