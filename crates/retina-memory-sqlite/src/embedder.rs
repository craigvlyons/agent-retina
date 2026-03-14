use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

pub(crate) enum Embedder {
    Fast(Box<TextEmbedding>),
    Fallback,
}

impl Embedder {
    pub(crate) fn new() -> Self {
        let mut options = InitOptions::default();
        options.model_name = EmbeddingModel::BGESmallENV15;
        options.show_download_progress = false;
        match TextEmbedding::try_new(options) {
            Ok(model) => Self::Fast(Box::new(model)),
            Err(_) => Self::Fallback,
        }
    }

    pub(crate) fn embed(&self, input: &str) -> Vec<f32> {
        match self {
            Self::Fast(model) => model
                .embed(vec![input], None)
                .ok()
                .and_then(|vectors| vectors.into_iter().next())
                .unwrap_or_else(|| hashed_embedding(input)),
            Self::Fallback => hashed_embedding(input),
        }
    }
}

fn hashed_embedding(input: &str) -> Vec<f32> {
    let digest = blake3::hash(input.as_bytes());
    let bytes = digest.as_bytes();
    (0..384)
        .map(|index| {
            let byte = bytes[index % bytes.len()];
            (byte as f32 / 255.0) * 2.0 - 1.0
        })
        .collect()
}
