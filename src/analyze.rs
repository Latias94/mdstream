use std::collections::HashMap;

use crate::syntax::{is_code_fence_closing_line, parse_code_fence_header_from_block};
use crate::types::BlockStatus;
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
        if update.reset {
            self.analyzer.reset();
            self.committed_meta.clear();
        }
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathMeta {
    pub balanced: bool,
}

#[derive(Debug, Default, Clone)]
pub struct MathAnalyzer;

fn count_double_dollars_unescaped(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut count = 0usize;
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'$' && bytes[i + 1] == b'$' {
            if i > 0 && bytes[i - 1] == b'\\' {
                i += 2;
                continue;
            }
            count += 1;
            i += 2;
            continue;
        }
        i += 1;
    }
    count
}

impl BlockAnalyzer for MathAnalyzer {
    type Meta = MathMeta;

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta> {
        if block.kind != BlockKind::MathBlock {
            return None;
        }
        let count = count_double_dollars_unescaped(&block.raw);
        Some(MathMeta {
            balanced: count % 2 == 0,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHintMeta {
    pub flags: u32,
}

impl BlockHintMeta {
    pub const DISPLAY_TRANSFORMED: u32 = 1 << 0;
    pub const UNCLOSED_CODE_FENCE: u32 = 1 << 1;
    pub const UNBALANCED_MATH: u32 = 1 << 2;

    pub fn likely_incomplete(&self) -> bool {
        self.flags != 0
    }

    pub fn has(&self, flag: u32) -> bool {
        (self.flags & flag) != 0
    }
}

#[derive(Debug, Default, Clone)]
pub struct BlockHintAnalyzer;

fn last_nonempty_line(text: &str) -> Option<&str> {
    text.split('\n').rev().find(|line| !line.trim().is_empty())
}

fn code_fence_is_closed(text: &str) -> bool {
    let Some(header) = parse_code_fence_header_from_block(text) else {
        return false;
    };
    let Some(last) = last_nonempty_line(text) else {
        return false;
    };
    is_code_fence_closing_line(last, header.fence_char, header.fence_len)
}

impl BlockAnalyzer for BlockHintAnalyzer {
    type Meta = BlockHintMeta;

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta> {
        if block.status != BlockStatus::Pending {
            return None;
        }

        let mut flags = 0u32;

        if let Some(display) = &block.display {
            if display != &block.raw {
                flags |= BlockHintMeta::DISPLAY_TRANSFORMED;
            }
        }

        match block.kind {
            BlockKind::CodeFence => {
                if !code_fence_is_closed(&block.raw) {
                    flags |= BlockHintMeta::UNCLOSED_CODE_FENCE;
                }
            }
            BlockKind::MathBlock => {
                let count = count_double_dollars_unescaped(&block.raw);
                if count % 2 == 1 {
                    flags |= BlockHintMeta::UNBALANCED_MATH;
                }
            }
            _ => {}
        }

        Some(BlockHintMeta { flags })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedBlockMeta {
    pub tag: String,
    pub attributes: Option<String>,
    pub closed: bool,
    /// Raw content between the opening/closing tag lines.
    ///
    /// If the closing tag is not present yet (pending), this includes everything after the opening
    /// tag line.
    pub content: String,
}

#[derive(Debug, Clone)]
pub struct TaggedBlockAnalyzer {
    /// If set, only tags in this allowlist produce meta.
    pub allowed_tags: Option<Vec<String>>,
    pub case_insensitive: bool,
}

impl Default for TaggedBlockAnalyzer {
    fn default() -> Self {
        Self {
            allowed_tags: None,
            case_insensitive: true,
        }
    }
}

fn custom_tag_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b':'
}

fn parse_custom_opening_tag(
    line: &str,
    case_insensitive: bool,
) -> Option<(String, Option<String>)> {
    let s = line.trim_start();
    if !s.starts_with('<') || s.starts_with("</") {
        return None;
    }
    let gt = s.find('>')?;
    let inside = &s[1..gt];
    let bytes = inside.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return None;
    }
    let mut name_end = 1usize;
    while name_end < bytes.len() && custom_tag_name_char(bytes[name_end]) {
        name_end += 1;
    }
    let mut name = inside[..name_end].to_string();
    if case_insensitive {
        name = name.to_ascii_lowercase();
    }
    let attrs = inside[name_end..].trim();
    let attrs = if attrs.is_empty() {
        None
    } else {
        Some(attrs.to_string())
    };
    Some((name, attrs))
}

fn is_custom_closing_tag(line: &str, tag: &str, case_insensitive: bool) -> bool {
    let s = line.trim_start();
    if !s.starts_with("</") {
        return false;
    }
    let Some(gt) = s.find('>') else {
        return false;
    };
    let inside = &s[2..gt];
    let bytes = inside.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_alphabetic() {
        return false;
    }
    let mut name_end = 1usize;
    while name_end < bytes.len() && custom_tag_name_char(bytes[name_end]) {
        name_end += 1;
    }
    let mut name = inside[..name_end].to_string();
    if case_insensitive {
        name = name.to_ascii_lowercase();
    }
    if name != tag {
        return false;
    }
    // Standalone closing tag line.
    inside[name_end..].trim().is_empty()
}

fn split_tag_block_content(raw: &str, tag: &str, case_insensitive: bool) -> (bool, String) {
    let mut lines: Vec<&str> = raw.split_inclusive('\n').collect();
    if lines.is_empty() {
        return (false, String::new());
    }

    // Remove opening tag line.
    let _ = lines.remove(0);

    // Remove closing tag line if present (last nonempty line).
    let mut last_nonempty_idx: Option<usize> = None;
    for (i, l) in lines.iter().enumerate().rev() {
        if !l.trim().is_empty() {
            last_nonempty_idx = Some(i);
            break;
        }
    }

    let mut closed = false;
    if let Some(idx) = last_nonempty_idx {
        let line = lines[idx];
        let line_no_nl = line.strip_suffix('\n').unwrap_or(line);
        if is_custom_closing_tag(line_no_nl, tag, case_insensitive) {
            closed = true;
            lines.remove(idx);
        }
    }

    (closed, lines.concat())
}

impl BlockAnalyzer for TaggedBlockAnalyzer {
    type Meta = TaggedBlockMeta;

    fn analyze_block(&mut self, block: &Block) -> Option<Self::Meta> {
        // Only consider blocks whose first line looks like an opening custom tag.
        let first_line = block.raw.split('\n').next().unwrap_or(&block.raw);
        let (tag, attrs) = parse_custom_opening_tag(first_line, self.case_insensitive)?;

        if let Some(allowed) = &self.allowed_tags {
            if !allowed.iter().any(|t| {
                if self.case_insensitive {
                    t.to_ascii_lowercase() == tag
                } else {
                    t == &tag
                }
            }) {
                return None;
            }
        }

        let (closed, content) = split_tag_block_content(&block.raw, &tag, self.case_insensitive);
        Some(TaggedBlockMeta {
            tag,
            attributes: attrs,
            closed,
            content,
        })
    }
}
