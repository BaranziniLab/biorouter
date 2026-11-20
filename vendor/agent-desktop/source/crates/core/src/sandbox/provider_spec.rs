#[derive(Debug)]
pub(crate) struct ProviderSpec {
    pub(crate) name: &'static str,
    pub(crate) files: &'static [(&'static str, &'static str)],
}
