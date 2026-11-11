use std::fmt::Display;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("base de données : {0}")]
    Db(#[from] rusqlite::Error),
    #[error("entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
    #[error("image illisible : {0}")]
    Image(#[from] image::ImageError),
    #[error("inférence : {0}")]
    Inference(String),
    #[error("modèle manquant ou modifié : {0}")]
    Model(String),
    #[error("{0}")]
    Invalid(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn inference(e: impl Display) -> Self {
        Error::Inference(e.to_string())
    }
}

impl From<ort::Error> for Error {
    fn from(e: ort::Error) -> Self {
        Error::Inference(e.to_string())
    }
}
