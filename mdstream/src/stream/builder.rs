use super::MdStream;
use crate::boundary::BoundaryPlugin;
use crate::options::Options;
use crate::transform::PendingTransformer;
use crate::transform::{IncompleteImageDropTransformer, IncompleteLinkPlaceholderTransformer};

#[derive(Debug)]
pub struct MdStreamBuilder {
    stream: MdStream,
}

impl MdStreamBuilder {
    pub fn new(opts: Options) -> Self {
        Self {
            stream: MdStream::new(opts),
        }
    }

    pub fn streamdown_defaults() -> Self {
        let opts = Options {
            terminator: crate::pending::TerminatorOptions {
                links: false,
                images: false,
                ..Default::default()
            },
            ..Default::default()
        };

        Self::new(opts.clone())
            .pending_transformer(IncompleteLinkPlaceholderTransformer {
                incomplete_link_url: opts.terminator.incomplete_link_url,
                window_bytes: opts.terminator_window_bytes,
            })
            .pending_transformer(IncompleteImageDropTransformer {
                window_bytes: opts.terminator_window_bytes,
            })
    }

    pub fn pending_transformer<T>(mut self, transformer: T) -> Self
    where
        T: PendingTransformer + 'static,
    {
        self.stream.push_pending_transformer(transformer);
        self
    }

    pub fn boundary_plugin<T>(mut self, plugin: T) -> Self
    where
        T: BoundaryPlugin + 'static,
    {
        self.stream.push_boundary_plugin(plugin);
        self
    }

    pub fn build(self) -> MdStream {
        self.stream
    }
}

impl Default for MdStreamBuilder {
    fn default() -> Self {
        Self::new(Options::default())
    }
}
