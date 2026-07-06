use crate::transform::PendingTransformer;

#[derive(Default)]
pub(crate) struct PendingTransformers {
    chain: Vec<Box<dyn PendingTransformer>>,
}

impl std::fmt::Debug for PendingTransformers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingTransformers")
            .field("len", &self.chain.len())
            .finish()
    }
}

impl PendingTransformers {
    pub(crate) fn push<T>(&mut self, transformer: T)
    where
        T: PendingTransformer + 'static,
    {
        self.chain.push(Box::new(transformer));
    }

    pub(crate) fn as_mut_slice(&mut self) -> &mut [Box<dyn PendingTransformer>] {
        &mut self.chain
    }

    pub(crate) fn reset_all(&mut self) {
        for transformer in &mut self.chain {
            transformer.reset();
        }
    }
}
