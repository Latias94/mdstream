use std::collections::HashMap;

use crate::syntax::parse_code_fence_header_from_block;
use crate::types::{Block, BlockId, BlockKind, Update};
use crate::{MdStream, Options};

pub trait BlockAnalyzer {
    type Meta: Clone;

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta>;

    fn reset(&mut self) {}
}

impl BlockAnalyzer for () {
    type Meta = ();

    fn analyze_block(&mut self, _block: &Block) -> Option<Self::Meta> {
        None
    }
}

impl<A, B> BlockAnalyzer for (A, B)
where
    A: BlockAnalyzer,
    B: BlockAnalyzer,
{
    type Meta = (Option<A::Meta>, Option<B::Meta>);

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta> {
        let a = self.0.analyze_block(block);
        let b = self.1.analyze_block(block);
        if a.is_none() && b.is_none() {
            None
        } else {
            Some((a, b))
        }
    }

    fn reset(&mut self) {
        self.0.reset();
        self.1.reset();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockMeta<M> {
    pub id: BlockId,
    pub meta: M,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnalyzedUpdate<M> {
    pub update: Update,
    pub committed_meta: Vec<BlockMeta<M>>,
    pub pending_meta: Option<BlockMeta<M>>,
}

impl<M> AnalyzedUpdate<M> {
    fn empty(update: Update) -> Self {
        Self {
            update,
            committed_meta: Vec::new(),
            pending_meta: None,
        }
    }
}

pub struct AnalyzedStream<A>
where
    A: BlockAnalyzer,
{
    inner: MdStream,
    analyzer: A,
    committed_meta: HashMap<BlockId, A::Meta>,
}

impl<A> AnalyzedStream<A>
where
    A: BlockAnalyzer,
{
    pub fn new(opts: Options, analyzer: A) -> Self {
        Self {
            inner: MdStream::new(opts),
            analyzer,
            committed_meta: HashMap::new(),
        }
    }

    pub fn inner(&self) -> &MdStream {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut MdStream {
        &mut self.inner
    }

    pub fn analyzer(&self) -> &A {
        &self.analyzer
    }

    pub fn analyzer_mut(&mut self) -> &mut A {
        &mut self.analyzer
    }

    pub fn meta_for(&self, id: BlockId) -> Option<&A::Meta> {
        self.committed_meta.get(&id)
    }

    pub fn append(&mut self, chunk: &str) -> AnalyzedUpdate<A::Meta> {
        let update = self.inner.append(chunk);
        self.analyze_update(update)
    }

    pub fn finalize(&mut self) -> AnalyzedUpdate<A::Meta> {
        let update = self.inner.finalize();
        self.analyze_update(update)
    }

    pub fn reset(&mut self) {
        self.inner.reset();
        self.analyzer.reset();
        self.committed_meta.clear();
    }

    fn analyze_update(&mut self, update: Update) -> AnalyzedUpdate<A::Meta> {
        let mut out = AnalyzedUpdate::empty(update);

        for block in &out.update.committed {
            let Some(meta) = self.analyzer.analyze_block(block) else {
                continue;
            };
            self.committed_meta.insert(block.id, meta.clone());
            out.committed_meta.push(BlockMeta { id: block.id, meta });
        }

        if let Some(pending) = &out.update.pending {
            if let Some(meta) = self.analyzer.analyze_block(pending) {
                out.pending_meta = Some(BlockMeta {
                    id: pending.id,
                    meta,
                });
            }
        }

        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeFenceClass {
    Mermaid,
    Json,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeFenceMeta {
    pub info: String,
    pub language: Option<String>,
    pub class: CodeFenceClass,
}

#[derive(Debug, Default, Clone)]
pub struct CodeFenceAnalyzer;

impl CodeFenceAnalyzer {
    fn classify_language(language: Option<&str>) -> CodeFenceClass {
        let Some(lang) = language else {
            return CodeFenceClass::Other;
        };
        let l = lang.to_ascii_lowercase();
        match l.as_str() {
            "mermaid" => CodeFenceClass::Mermaid,
            "json" | "jsonc" | "json5" | "jsonl" | "jsonp" => CodeFenceClass::Json,
            _ => CodeFenceClass::Other,
        }
    }
}

impl BlockAnalyzer for CodeFenceAnalyzer {
    type Meta = CodeFenceMeta;

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta> {
        if block.kind != BlockKind::CodeFence {
            return None;
        }
        let header = parse_code_fence_header_from_block(&block.raw)?;
        Some(CodeFenceMeta {
            info: header.info.to_string(),
            language: header.language.map(|s| s.to_string()),
            class: Self::classify_language(header.language),
        })
    }
}

