use crate::{
    BoundaryPlugin, IncompleteImageDropTransformer, IncompleteLinkPlaceholderTransformer, Options,
    PendingTransformer, StreamEngine,
};

#[derive(Debug)]
pub struct StreamEngineBuilder {
    engine: StreamEngine,
}

impl StreamEngineBuilder {
    pub fn new(options: Options) -> Self {
        Self {
            engine: StreamEngine::new(options),
        }
    }

    pub fn streamdown_defaults() -> Self {
        let options = Options {
            terminator: crate::pending::TerminatorOptions {
                links: false,
                images: false,
                ..Default::default()
            },
            ..Default::default()
        };
        Self::new(options.clone())
            .pending_transformer(IncompleteLinkPlaceholderTransformer {
                incomplete_link_url: options.terminator.incomplete_link_url,
                window_bytes: options.terminator_window_bytes,
            })
            .pending_transformer(IncompleteImageDropTransformer {
                window_bytes: options.terminator_window_bytes,
            })
    }

    pub fn pending_transformer<T>(mut self, transformer: T) -> Self
    where
        T: PendingTransformer + 'static,
    {
        self.engine.push_pending_transformer_legacy(transformer);
        self
    }

    pub fn boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.engine.push_boundary_plugin_legacy(plugin);
        self
    }

    pub fn build(self) -> StreamEngine {
        self.engine
    }
}

impl Default for StreamEngineBuilder {
    fn default() -> Self {
        Self::new(Options::default())
    }
}
