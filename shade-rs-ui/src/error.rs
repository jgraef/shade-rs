#[derive(Debug, thiserror::Error)]
#[error("shade-rs-ui error")]
pub enum Error {
    Graphics(#[from] crate::graphics::Error),
}
