use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use mdstream_protocol::{
    ChildListOwner, ContentKind, ContentNode, ContinuityGeneration, Document, NodeId,
    NodeStability, NodeVersion, SemanticText, SourceRange, TransitionChildListOwner,
    TransitionFacts, TransitionNodeKey,
};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

use super::presentation::RootProjection;

const MAX_HIGHLIGHT_BYTES: usize = 64 * 1024;
const MAX_HIGHLIGHT_CACHE_BYTES: usize = 256 * 1024;
const MAX_HIGHLIGHT_CACHE_ENTRIES: usize = 1024;
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "function",
    "function.builtin",
    "keyword",
    "module",
    "number",
    "operator",
    "property",
    "punctuation",
    "string",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

type UiLine = Line<'static>;

#[derive(Debug, Clone, PartialEq)]
pub(super) struct ProjectionSet {
    pub(super) stable: Vec<RootProjection>,
    pub(super) mutable: Vec<RootProjection>,
}

#[derive(Debug, Clone)]
struct CachedRootProjection {
    projection: RootProjection,
    highlighted_segments: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ProjectionMetrics {
    pub(super) stable_roots_rendered: u64,
    pub(super) stable_roots_reused: u64,
    pub(super) mutable_roots_rendered: u64,
    pub(super) full_invalidations: u64,
}

#[derive(Debug, Default)]
pub(super) struct ProjectionCache {
    stable: BTreeMap<TransitionNodeKey, CachedRootProjection>,
    node_roots: HashMap<TransitionNodeKey, TransitionNodeKey>,
    dirty_roots: HashSet<TransitionNodeKey>,
    metrics: ProjectionMetrics,
}

impl ProjectionCache {
    pub(super) fn observe_facts(
        &mut self,
        facts: &TransitionFacts,
        document: Option<&Document>,
        continuity_generation: ContinuityGeneration,
    ) {
        match facts {
            TransitionFacts::FullReplace { .. } => {
                self.stable.clear();
                self.node_roots.clear();
                self.dirty_roots.clear();
                self.metrics.full_invalidations = self.metrics.full_invalidations.saturating_add(1);
            }
            TransitionFacts::Continuous {
                nodes,
                structures,
                resources,
                ..
            } => {
                let mut uncertain = false;
                for node in nodes {
                    uncertain |= !self.mark_node(node.key, document, continuity_generation);
                }
                for structure in structures {
                    if let TransitionChildListOwner::Node { key } = structure.owner {
                        uncertain |= !self.mark_node(key, document, continuity_generation);
                    }
                    for key in structure.removed.iter().chain(&structure.inserted) {
                        uncertain |= !self.mark_node(*key, document, continuity_generation);
                    }
                }
                for resource in resources {
                    for key in &resource.affected_nodes {
                        uncertain |= !self.mark_node(*key, document, continuity_generation);
                    }
                }
                if uncertain {
                    self.dirty_roots.extend(self.stable.keys().copied());
                    self.metrics.full_invalidations =
                        self.metrics.full_invalidations.saturating_add(1);
                }
            }
        }
    }

    pub(super) const fn metrics(&self) -> ProjectionMetrics {
        self.metrics
    }

    fn mark_node(
        &mut self,
        key: TransitionNodeKey,
        document: Option<&Document>,
        continuity_generation: ContinuityGeneration,
    ) -> bool {
        let mut found = false;
        if let Some(root) = self.node_roots.get(&key).copied() {
            self.dirty_roots.insert(root);
            found = true;
        }
        if let Some(document) = document
            && key.continuity_generation == continuity_generation
            && key.epoch == document.coordinate().epoch
            && let Some(root) = root_key_for_node(document, continuity_generation, key.node_id)
        {
            self.dirty_roots.insert(root);
            found = true;
        }
        found
    }

    fn rebuild_node_roots(
        &mut self,
        document: &Document,
        continuity_generation: ContinuityGeneration,
    ) {
        self.node_roots.clear();
        let epoch = document.coordinate().epoch;
        for root_id in document.roots().iter().copied() {
            let root = TransitionNodeKey {
                continuity_generation,
                epoch,
                node_id: root_id,
            };
            let mut pending = vec![root_id];
            while let Some(node_id) = pending.pop() {
                let Some(node) = document.node(node_id) else {
                    continue;
                };
                self.node_roots.insert(
                    TransitionNodeKey {
                        continuity_generation,
                        epoch,
                        node_id,
                    },
                    root,
                );
                pending.extend(node.children.iter().copied());
            }
        }
    }
}

pub(super) fn project_document(
    document: &Document,
    continuity_generation: ContinuityGeneration,
    cache: &mut ProjectionCache,
    syntax: Option<&mut SyntaxHighlighter>,
) -> ProjectionSet {
    let roots = document.roots().as_slice();
    let stable_prefix_len = stable_root_prefix_len(document);
    let epoch = document.coordinate().epoch;
    let mut syntax = syntax;

    if let Some(syntax) = syntax.as_deref_mut() {
        syntax.begin_render();
    }

    let mut stable = Vec::with_capacity(stable_prefix_len);
    let mut mutable = Vec::with_capacity(roots.len().saturating_sub(stable_prefix_len));
    let mut next_stable_cache = BTreeMap::new();
    for (index, root_id) in roots.iter().copied().enumerate() {
        if document.node(root_id).is_none() {
            continue;
        }

        let projection_is_stable = index < stable_prefix_len;
        let key = TransitionNodeKey {
            continuity_generation,
            epoch,
            node_id: root_id,
        };
        if projection_is_stable
            && !cache.dirty_roots.contains(&key)
            && let Some(cached) = cache.stable.get(&key).cloned()
        {
            if let Some(syntax) = syntax.as_deref_mut() {
                syntax.add_render_segments(cached.highlighted_segments);
            }
            cache.metrics.stable_roots_reused = cache.metrics.stable_roots_reused.saturating_add(1);
            stable.push(cached.projection.clone());
            next_stable_cache.insert(key, cached);
            continue;
        }

        let segments_before = syntax
            .as_deref()
            .map_or(0, SyntaxHighlighter::render_segments);
        let mut lines = Vec::new();
        render_blocks(
            document,
            std::slice::from_ref(&root_id),
            continuity_generation,
            projection_is_stable,
            syntax.as_deref_mut(),
            &mut lines,
        );
        let projection = RootProjection::new(key, lines);
        if projection_is_stable {
            let highlighted_segments = syntax
                .as_deref()
                .map_or(0, SyntaxHighlighter::render_segments)
                .saturating_sub(segments_before);
            cache.metrics.stable_roots_rendered =
                cache.metrics.stable_roots_rendered.saturating_add(1);
            next_stable_cache.insert(
                key,
                CachedRootProjection {
                    projection: projection.clone(),
                    highlighted_segments,
                },
            );
            stable.push(projection);
        } else {
            cache.metrics.mutable_roots_rendered =
                cache.metrics.mutable_roots_rendered.saturating_add(1);
            mutable.push(projection);
        }
    }

    cache.stable = next_stable_cache;
    cache.dirty_roots.clear();
    cache.rebuild_node_roots(document, continuity_generation);
    ProjectionSet { stable, mutable }
}

fn root_key_for_node(
    document: &Document,
    continuity_generation: ContinuityGeneration,
    mut node_id: NodeId,
) -> Option<TransitionNodeKey> {
    loop {
        match document.parent(node_id)? {
            ChildListOwner::Document => {
                return Some(TransitionNodeKey {
                    continuity_generation,
                    epoch: document.coordinate().epoch,
                    node_id,
                });
            }
            ChildListOwner::Node { node_id: parent } => node_id = parent,
        }
    }
}

pub(super) fn flatten_projections(projections: &ProjectionSet) -> Vec<UiLine> {
    projections
        .stable
        .iter()
        .chain(&projections.mutable)
        .flat_map(|projection| projection.lines().iter().cloned())
        .collect()
}

fn stable_root_prefix_len(document: &Document) -> usize {
    stable_root_prefix_len_with(document.roots().as_slice(), &|node_id| {
        document.node(node_id)
    })
}

fn stable_root_prefix_len_with<'a>(
    roots: &[NodeId],
    resolve: &impl Fn(NodeId) -> Option<&'a ContentNode>,
) -> usize {
    roots
        .iter()
        .take_while(|node_id| subtree_is_stable_with(**node_id, resolve))
        .count()
}

fn subtree_is_stable(document: &Document, root: NodeId) -> bool {
    subtree_is_stable_with(root, &|node_id| document.node(node_id))
}

fn subtree_is_stable_with<'a>(
    root: NodeId,
    resolve: &impl Fn(NodeId) -> Option<&'a ContentNode>,
) -> bool {
    let mut pending = vec![root];
    while let Some(node_id) = pending.pop() {
        let Some(node) = resolve(node_id) else {
            return false;
        };
        if node.stability != NodeStability::Stable {
            return false;
        }
        pending.extend(node.children.iter().copied());
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodeLanguage {
    Rust,
    Json,
}

impl CodeLanguage {
    fn from_fence(language: Option<&str>) -> Option<Self> {
        match language?.to_ascii_lowercase().as_str() {
            "rust" | "rs" => Some(Self::Rust),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

#[derive(Clone)]
struct CachedCode {
    version: NodeVersion,
    language: CodeLanguage,
    source: String,
    lines: Vec<UiLine>,
    segments: usize,
    last_used: u64,
}

impl CachedCode {
    fn bytes(&self) -> usize {
        self.source.len()
    }
}

pub(super) struct SyntaxHighlighter {
    rust: HighlightConfiguration,
    json: HighlightConfiguration,
    highlighter: Highlighter,
    cache: BTreeMap<TransitionNodeKey, CachedCode>,
    lru: BTreeSet<(u64, TransitionNodeKey)>,
    cache_bytes: usize,
    access_clock: u64,
    render_segments: usize,
}

impl SyntaxHighlighter {
    pub(super) fn new() -> Option<Self> {
        let mut rust = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        )
        .ok()?;
        rust.configure(HIGHLIGHT_NAMES);

        let mut json = HighlightConfiguration::new(
            tree_sitter_json::LANGUAGE.into(),
            "json",
            tree_sitter_json::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .ok()?;
        json.configure(HIGHLIGHT_NAMES);

        Some(Self {
            rust,
            json,
            highlighter: Highlighter::new(),
            cache: BTreeMap::new(),
            lru: BTreeSet::new(),
            cache_bytes: 0,
            access_clock: 0,
            render_segments: 0,
        })
    }

    pub(super) fn clear(&mut self) {
        self.cache.clear();
        self.lru.clear();
        self.cache_bytes = 0;
        self.access_clock = 0;
        self.render_segments = 0;
    }

    pub(super) fn invalidate_key(&mut self, key: TransitionNodeKey) {
        if let Some(cached) = self.cache.remove(&key) {
            self.lru.remove(&(cached.last_used, key));
            self.cache_bytes = self.cache_bytes.saturating_sub(cached.bytes());
        }
    }

    pub(super) fn begin_render(&mut self) {
        self.render_segments = 0;
    }

    pub(super) const fn render_segments(&self) -> usize {
        self.render_segments
    }

    #[cfg(test)]
    pub(super) fn cache_len(&self) -> usize {
        self.cache.len()
    }

    fn add_render_segments(&mut self, segments: usize) {
        self.render_segments = self.render_segments.saturating_add(segments);
    }

    fn render_code(
        &mut self,
        key: TransitionNodeKey,
        version: &NodeVersion,
        language: Option<&str>,
        source: &str,
        stable: bool,
    ) -> Vec<UiLine> {
        if !stable {
            return plain_code_lines(source);
        }
        let Some(language) = CodeLanguage::from_fence(language) else {
            return plain_code_lines(source);
        };
        if source.is_empty() || source.len() > MAX_HIGHLIGHT_BYTES {
            return plain_code_lines(source);
        }
        if let Some(lines) = self.cached_lines(key, version, language, source) {
            return lines;
        }

        let Some((lines, segments)) = self.highlight(language, source) else {
            return plain_code_lines(source);
        };
        self.render_segments = self.render_segments.saturating_add(segments);
        self.insert_cache(
            key,
            version.clone(),
            language,
            source.to_string(),
            lines.clone(),
            segments,
        );
        lines
    }

    fn cached_lines(
        &mut self,
        key: TransitionNodeKey,
        version: &NodeVersion,
        language: CodeLanguage,
        source: &str,
    ) -> Option<Vec<UiLine>> {
        let previous_access = match self.cache.get(&key) {
            Some(cached)
                if cached.version == *version
                    && cached.language == language
                    && cached.source.as_str() == source =>
            {
                cached.last_used
            }
            Some(_) => {
                self.invalidate_key(key);
                return None;
            }
            None => return None,
        };

        let last_used = self.next_access();
        self.lru.remove(&(previous_access, key));
        let cached = self
            .cache
            .get_mut(&key)
            .expect("a validated cache hit remains present");
        cached.last_used = last_used;
        self.render_segments = self.render_segments.saturating_add(cached.segments);
        let lines = cached.lines.clone();
        self.lru.insert((last_used, key));
        Some(lines)
    }

    fn insert_cache(
        &mut self,
        key: TransitionNodeKey,
        version: NodeVersion,
        language: CodeLanguage,
        source: String,
        lines: Vec<UiLine>,
        segments: usize,
    ) {
        if source.is_empty() || source.len() > MAX_HIGHLIGHT_BYTES {
            return;
        }

        self.invalidate_key(key);
        let last_used = self.next_access();
        let cached = CachedCode {
            version,
            language,
            source,
            lines,
            segments,
            last_used,
        };
        self.cache_bytes = self.cache_bytes.saturating_add(cached.bytes());
        self.cache.insert(key, cached);
        self.lru.insert((last_used, key));

        while self.cache_bytes > MAX_HIGHLIGHT_CACHE_BYTES
            || self.cache.len() > MAX_HIGHLIGHT_CACHE_ENTRIES
        {
            let Some((_, lru_key)) = self.lru.first().copied() else {
                break;
            };
            self.invalidate_key(lru_key);
        }
    }

    fn next_access(&mut self) -> u64 {
        if self.access_clock == u64::MAX {
            self.rebase_access_clock();
        }
        self.access_clock = self.access_clock.saturating_add(1);
        self.access_clock
    }

    fn rebase_access_clock(&mut self) {
        let keys = self.lru.iter().map(|(_, key)| *key).collect::<Vec<_>>();

        self.access_clock = 0;
        self.lru.clear();
        for key in keys {
            self.access_clock = self.access_clock.saturating_add(1);
            if let Some(cached) = self.cache.get_mut(&key) {
                cached.last_used = self.access_clock;
                self.lru.insert((self.access_clock, key));
            }
        }
    }

    fn highlight(&mut self, language: CodeLanguage, source: &str) -> Option<(Vec<UiLine>, usize)> {
        let (highlighter, configuration) = match language {
            CodeLanguage::Rust => (&mut self.highlighter, &self.rust),
            CodeLanguage::Json => (&mut self.highlighter, &self.json),
        };
        let events = highlighter
            .highlight(configuration, source.as_bytes(), None, |_| None)
            .ok()?;
        let mut lines = vec![Vec::new()];
        let mut styles = vec![Style::default().fg(Color::White)];
        let mut segments = 0_usize;

        for event in events {
            match event.ok()? {
                HighlightEvent::HighlightStart(Highlight(index)) => {
                    styles.push(highlight_style(index));
                    segments = segments.saturating_add(1);
                }
                HighlightEvent::HighlightEnd => {
                    if styles.len() > 1 {
                        styles.pop();
                    }
                }
                HighlightEvent::Source { start, end } => {
                    let text = source.get(start..end)?;
                    append_styled_text(
                        &mut lines,
                        text,
                        styles.last().copied().unwrap_or_else(Style::default),
                    );
                }
            }
        }

        Some((lines.into_iter().map(Line::from).collect(), segments))
    }
}

fn highlight_style(index: usize) -> Style {
    let name = HIGHLIGHT_NAMES.get(index).copied().unwrap_or_default();
    match name {
        "comment" => Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::ITALIC),
        "keyword" | "operator" => Style::default().fg(Color::Magenta),
        "string" => Style::default().fg(Color::Green),
        "number" | "constant" | "constant.builtin" => Style::default().fg(Color::Yellow),
        "type" | "type.builtin" | "constructor" => Style::default().fg(Color::Cyan),
        "function" | "function.builtin" => Style::default().fg(Color::Blue),
        "variable.parameter" => Style::default().fg(Color::LightMagenta),
        "property" | "attribute" => Style::default().fg(Color::LightCyan),
        "punctuation" => Style::default().fg(Color::Gray),
        _ => Style::default().fg(Color::White),
    }
}

fn plain_code_lines(source: &str) -> Vec<UiLine> {
    source
        .split('\n')
        .map(|line| {
            Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::Gray),
            ))
        })
        .collect()
}

fn append_styled_text(lines: &mut Vec<Vec<Span<'static>>>, text: &str, style: Style) {
    for fragment in text.split_inclusive('\n') {
        let (content, newline) = match fragment.strip_suffix('\n') {
            Some(content) => (content, true),
            None => (fragment, false),
        };
        if !content.is_empty() {
            lines
                .last_mut()
                .expect("highlight output always has an active line")
                .push(Span::styled(content.to_string(), style));
        }
        if newline {
            lines.push(Vec::new());
        }
    }
}

fn render_blocks(
    document: &Document,
    ids: &[NodeId],
    continuity_generation: ContinuityGeneration,
    projection_is_stable: bool,
    syntax: Option<&mut SyntaxHighlighter>,
    lines: &mut Vec<UiLine>,
) {
    let mut syntax = syntax;
    for id in ids {
        let Some(node) = document.node(*id) else {
            continue;
        };
        match &node.content {
            ContentKind::Heading { level } => {
                let mut spans = vec![Span::styled(
                    format!("{} ", "#".repeat((*level).into())),
                    heading_style(*level),
                )];
                inline_spans(
                    document,
                    node.children.as_slice(),
                    heading_style(*level),
                    &mut spans,
                );
                push_nonempty_line(lines, spans);
                lines.push(Line::default());
            }
            ContentKind::Paragraph {} => {
                let mut spans = Vec::new();
                inline_spans(
                    document,
                    node.children.as_slice(),
                    Style::default().fg(Color::White),
                    &mut spans,
                );
                push_nonempty_line(lines, spans);
                lines.push(Line::default());
            }
            ContentKind::List { ordered, start, .. } => {
                render_list(
                    document,
                    node,
                    *ordered,
                    *start,
                    continuity_generation,
                    projection_is_stable,
                    syntax.as_deref_mut(),
                    lines,
                );
            }
            ContentKind::BlockQuote { .. } => {
                let mut quoted = Vec::new();
                render_blocks(
                    document,
                    node.children.as_slice(),
                    continuity_generation,
                    projection_is_stable,
                    syntax.as_deref_mut(),
                    &mut quoted,
                );
                for line in quoted {
                    lines.push(prefix_line(
                        Span::styled("│ ", Style::default().fg(Color::Cyan)),
                        line,
                    ));
                }
                lines.push(Line::default());
            }
            ContentKind::CodeBlock { text, .. } => {
                let language = node.content.code_language().unwrap_or("plain");
                let code = semantic_text(document, node, text);
                lines.push(Line::from(Span::styled(
                    format!(" {language} "),
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                let code_is_stable = projection_is_stable && subtree_is_stable(document, node.id);
                let key = TransitionNodeKey {
                    continuity_generation,
                    epoch: document.coordinate().epoch,
                    node_id: node.id,
                };
                let code_lines = match syntax.as_deref_mut() {
                    Some(syntax) => syntax.render_code(
                        key,
                        &node.version,
                        Some(language),
                        &code,
                        code_is_stable,
                    ),
                    None => plain_code_lines(&code),
                };
                for code_line in code_lines {
                    lines.push(prefix_line(
                        Span::styled("  ", Style::default().fg(Color::DarkGray)),
                        code_line,
                    ));
                }
                lines.push(Line::default());
            }
            ContentKind::ThematicBreak {} => {
                lines.push(Line::from(Span::styled(
                    "----------------------------------------",
                    Style::default().fg(Color::DarkGray),
                )));
                lines.push(Line::default());
            }
            ContentKind::Math { text, display } => {
                let value = semantic_text(document, node, text);
                if !value.is_empty() {
                    let delimiters = if *display { ("$$", "$$") } else { ("$", "$") };
                    lines.push(Line::from(Span::styled(
                        format!("{}{}{}", delimiters.0, value, delimiters.1),
                        Style::default().fg(Color::Magenta),
                    )));
                    lines.push(Line::default());
                }
            }
            ContentKind::CitationDefinition { key, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("[citation definition: {key}]"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            ContentKind::FootnoteDefinition { label, .. } => {
                lines.push(Line::from(Span::styled(
                    format!("[footnote definition: {label}]"),
                    Style::default().fg(Color::DarkGray),
                )));
            }
            _ if !node.children.is_empty() => {
                render_blocks(
                    document,
                    node.children.as_slice(),
                    continuity_generation,
                    projection_is_stable,
                    syntax.as_deref_mut(),
                    lines,
                );
            }
            _ => {}
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_list(
    document: &Document,
    list: &ContentNode,
    ordered: bool,
    start: Option<u32>,
    continuity_generation: ContinuityGeneration,
    projection_is_stable: bool,
    syntax: Option<&mut SyntaxHighlighter>,
    lines: &mut Vec<UiLine>,
) {
    let mut syntax = syntax;
    for (index, item_id) in list.children.iter().enumerate() {
        let Some(item) = document.node(*item_id) else {
            continue;
        };
        let mut item_lines = Vec::new();
        if matches!(item.content, ContentKind::ListItem { .. }) {
            render_blocks(
                document,
                item.children.as_slice(),
                continuity_generation,
                projection_is_stable,
                syntax.as_deref_mut(),
                &mut item_lines,
            );
        } else {
            render_blocks(
                document,
                std::slice::from_ref(item_id),
                continuity_generation,
                projection_is_stable,
                syntax.as_deref_mut(),
                &mut item_lines,
            );
        }

        let marker = list_marker(item, ordered, start, index);
        let prefix = Span::styled(marker, Style::default().fg(Color::Cyan));
        if let Some(first) = item_lines.first_mut() {
            first.spans.insert(0, prefix);
        } else {
            item_lines.push(Line::from(prefix));
        }
        if item_lines.last().is_some_and(|line| line.spans.is_empty()) {
            item_lines.pop();
        }
        lines.append(&mut item_lines);
    }
    lines.push(Line::default());
}

fn list_marker(item: &ContentNode, ordered: bool, start: Option<u32>, index: usize) -> String {
    let task_prefix = match item.content {
        ContentKind::ListItem {
            checked: Some(true),
        } => "[x] ",
        ContentKind::ListItem {
            checked: Some(false),
        } => "[ ] ",
        _ => "",
    };
    if ordered {
        let number = start.unwrap_or(1).saturating_add(index as u32);
        format!("{number}. {task_prefix}")
    } else {
        format!("- {task_prefix}")
    }
}

fn inline_spans(
    document: &Document,
    ids: &[NodeId],
    inherited_style: Style,
    spans: &mut Vec<Span<'static>>,
) {
    for id in ids {
        let Some(node) = document.node(*id) else {
            continue;
        };
        match &node.content {
            ContentKind::Text { text } => {
                push_text_span(spans, semantic_text(document, node, text), inherited_style);
            }
            ContentKind::Emphasis {} => inline_spans(
                document,
                node.children.as_slice(),
                inherited_style.add_modifier(Modifier::ITALIC),
                spans,
            ),
            ContentKind::Strong {} => inline_spans(
                document,
                node.children.as_slice(),
                inherited_style.add_modifier(Modifier::BOLD),
                spans,
            ),
            ContentKind::Strikethrough {} => inline_spans(
                document,
                node.children.as_slice(),
                inherited_style.add_modifier(Modifier::CROSSED_OUT),
                spans,
            ),
            ContentKind::Link { .. } => inline_spans(
                document,
                node.children.as_slice(),
                inherited_style
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::UNDERLINED),
                spans,
            ),
            ContentKind::InlineCode { text } => push_text_span(
                spans,
                semantic_text(document, node, text),
                inherited_style.fg(Color::Yellow).bg(Color::DarkGray),
            ),
            ContentKind::Image { alt, .. } => push_text_span(
                spans,
                format!("[image: {}]", semantic_text(document, node, alt)),
                inherited_style.fg(Color::Cyan),
            ),
            ContentKind::CitationReference { key, .. } => push_text_span(
                spans,
                format!("[{key}]"),
                inherited_style.fg(Color::Magenta),
            ),
            ContentKind::FootnoteReference { label, .. } => push_text_span(
                spans,
                format!("[^{label}]"),
                inherited_style.fg(Color::Magenta),
            ),
            ContentKind::SoftBreak {} | ContentKind::HardBreak {} => {
                push_text_span(spans, " ".to_string(), inherited_style);
            }
            ContentKind::Html { text, .. } => push_text_span(
                spans,
                semantic_text(document, node, text),
                inherited_style.fg(Color::DarkGray),
            ),
            ContentKind::Custom {
                namespace, name, ..
            } => {
                push_text_span(
                    spans,
                    format!("[{namespace}/{name}]"),
                    inherited_style.fg(Color::Blue),
                );
                inline_spans(document, node.children.as_slice(), inherited_style, spans);
            }
            _ if !node.children.is_empty() => {
                inline_spans(document, node.children.as_slice(), inherited_style, spans);
            }
            _ => {}
        }
    }
}

fn semantic_text(document: &Document, node: &ContentNode, text: &SemanticText) -> String {
    semantic_text_value(document.source(), node.body, text)
}

fn semantic_text_value(source: &str, body: SourceRange, text: &SemanticText) -> String {
    match text {
        SemanticText::Source {} => {
            let body_start = cursor_to_usize(body.start);
            let body_end = cursor_to_usize(body.end);
            source
                .get(body_start..body_end)
                .expect("canonical body ranges are UTF-8 boundaries")
                .to_string()
        }
        SemanticText::Normalized { value } => value.clone(),
    }
}

fn cursor_to_usize(cursor: mdstream_protocol::SourceCursor) -> usize {
    usize::try_from(cursor.get()).expect("canonical source cursors fit the process address space")
}

fn push_text_span(spans: &mut Vec<Span<'static>>, text: String, style: Style) {
    if !text.is_empty() {
        spans.push(Span::styled(text, style));
    }
}

fn push_nonempty_line(lines: &mut Vec<UiLine>, spans: Vec<Span<'static>>) {
    if spans.iter().any(|span| !span.content.is_empty()) {
        lines.push(Line::from(spans));
    }
}

fn prefix_line(prefix: Span<'static>, mut line: UiLine) -> UiLine {
    line.spans.insert(0, prefix);
    line
}

fn heading_style(level: u8) -> Style {
    match level {
        1 => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        2 => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
        _ => Style::default()
            .fg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use mdstream::{EngineOutput, StreamEngine};
    use mdstream_protocol::{
        ContentKind, ContentNode, ContinuityGeneration, Epoch, NodeStability, NodeVersion,
        SemanticText, SourceCursor, SourceRange, TransitionNodeKey, TransitionReducer,
    };

    use super::*;

    fn empty_range() -> SourceRange {
        SourceRange::new(SourceCursor::new(0), SourceCursor::new(0))
    }

    fn apply_output(reducer: &mut TransitionReducer, output: EngineOutput) {
        for change in output.into_changes() {
            reducer
                .apply(change)
                .expect("engine output must replay through the transition reducer");
        }
    }

    fn apply_output_and_observe(
        reducer: &mut TransitionReducer,
        projections: &mut ProjectionCache,
        output: EngineOutput,
    ) {
        for change in output.into_changes() {
            let outcome = reducer
                .apply(change)
                .expect("engine output must replay through the transition reducer");
            if let Some(facts) = outcome.facts.as_ref() {
                projections.observe_facts(
                    facts,
                    reducer.document(),
                    reducer.continuity_generation(),
                );
            }
        }
    }

    fn key(node_id: u64) -> TransitionNodeKey {
        TransitionNodeKey {
            continuity_generation: ContinuityGeneration::new(0),
            epoch: Epoch::new(0),
            node_id: NodeId::from(node_id),
        }
    }

    fn line_texts(projections: &ProjectionSet) -> Vec<String> {
        flatten_projections(projections)
            .iter()
            .map(|line| {
                line.spans.iter().fold(String::new(), |mut text, span| {
                    text.push_str(span.content.as_ref());
                    text
                })
            })
            .collect()
    }

    #[test]
    fn stable_parent_with_provisional_child_is_not_a_stable_subtree() {
        let parent_id = NodeId::from(1_u64);
        let child_id = NodeId::from(2_u64);
        let child = ContentNode::leaf(
            child_id,
            NodeStability::Provisional,
            empty_range(),
            ContentKind::Text {
                text: SemanticText::Normalized {
                    value: "pending".to_string(),
                },
            },
        );
        let parent = ContentNode::new(
            parent_id,
            NodeStability::Stable,
            empty_range(),
            empty_range(),
            vec![child_id],
            ContentKind::Paragraph {},
        );
        let nodes = BTreeMap::from([(parent_id, parent), (child_id, child)]);

        assert!(!subtree_is_stable_with(parent_id, &|id| nodes.get(&id)));
    }

    #[test]
    fn stable_prefix_stops_before_a_provisional_root() {
        let first_id = NodeId::from(1_u64);
        let provisional_id = NodeId::from(2_u64);
        let later_stable_id = NodeId::from(3_u64);
        let nodes = BTreeMap::from([
            (
                first_id,
                ContentNode::leaf(
                    first_id,
                    NodeStability::Stable,
                    empty_range(),
                    ContentKind::ThematicBreak {},
                ),
            ),
            (
                provisional_id,
                ContentNode::leaf(
                    provisional_id,
                    NodeStability::Provisional,
                    empty_range(),
                    ContentKind::Paragraph {},
                ),
            ),
            (
                later_stable_id,
                ContentNode::leaf(
                    later_stable_id,
                    NodeStability::Stable,
                    empty_range(),
                    ContentKind::ThematicBreak {},
                ),
            ),
        ]);
        let resolve = |id| nodes.get(&id);

        assert_eq!(
            stable_root_prefix_len_with(&[first_id, provisional_id, later_stable_id], &resolve,),
            1
        );
    }

    #[test]
    fn semantic_text_uses_the_complete_body_or_normalized_value() {
        let source = "xxsourceyy";
        let body = SourceRange::new(SourceCursor::new(2), SourceCursor::new(8));

        assert_eq!(
            semantic_text_value(source, body, &SemanticText::Source {}),
            "source"
        );
        assert_eq!(
            semantic_text_value(
                source,
                body,
                &SemanticText::Normalized {
                    value: "decoded & complete".to_string(),
                },
            ),
            "decoded & complete"
        );
    }

    #[test]
    fn mutable_code_is_plain_then_stable_code_preserves_text_topology() {
        let mut engine = StreamEngine::new();
        let mut reducer = TransitionReducer::new();
        let mut projections = ProjectionCache::default();
        let mut syntax = SyntaxHighlighter::new().expect("fixture grammars compile");

        apply_output(
            &mut reducer,
            engine
                .append("```rust\nfn main() {}\n")
                .expect("append must succeed"),
        );
        let mutable = project_document(
            reducer.document().expect("append creates a document"),
            reducer.continuity_generation(),
            &mut projections,
            Some(&mut syntax),
        );
        assert!(mutable.stable.is_empty());
        assert_eq!(mutable.mutable.len(), 1);
        assert!(syntax.cache.is_empty());
        assert_eq!(syntax.render_segments(), 0);
        let mutable_text = line_texts(&mutable);

        apply_output(
            &mut reducer,
            engine.finish().expect("finish must stabilize the document"),
        );
        let stable = project_document(
            reducer
                .document()
                .expect("finished document remains available"),
            reducer.continuity_generation(),
            &mut projections,
            Some(&mut syntax),
        );
        assert_eq!(stable.stable.len(), 1);
        assert!(stable.mutable.is_empty());
        assert_eq!(line_texts(&stable), mutable_text);
        assert_eq!(
            flatten_projections(&stable).len(),
            flatten_projections(&mutable).len()
        );
        assert_eq!(syntax.cache.len(), 1);
        assert!(syntax.render_segments() > 0);
    }

    #[test]
    fn unchanged_stable_roots_reuse_their_rendered_projection() {
        let mut engine = StreamEngine::new();
        let mut reducer = TransitionReducer::new();
        let mut projections = ProjectionCache::default();
        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine.append("# title\n\n").expect("append must succeed"),
        );
        let document = reducer.document().expect("append creates a document");

        let first = project_document(
            document,
            reducer.continuity_generation(),
            &mut projections,
            None,
        );
        let second = project_document(
            document,
            reducer.continuity_generation(),
            &mut projections,
            None,
        );

        assert_eq!(first, second);
        assert_eq!(projections.metrics().stable_roots_rendered, 1);
        assert_eq!(projections.metrics().stable_roots_reused, 1);
    }

    #[test]
    fn late_reference_correction_rerenders_only_the_affected_stable_root() {
        let mut engine = StreamEngine::new();
        let mut reducer = TransitionReducer::new();
        let mut projections = ProjectionCache::default();
        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine
                .append("[shared]\n\nunrelated\n\n")
                .expect("initial append must succeed"),
        );
        let before = project_document(
            reducer.document().expect("append creates a document"),
            reducer.continuity_generation(),
            &mut projections,
            None,
        );
        let before_metrics = projections.metrics();

        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine
                .append("[shared]: /target\n")
                .expect("definition append must succeed"),
        );
        let after = project_document(
            reducer.document().expect("correction retains a document"),
            reducer.continuity_generation(),
            &mut projections,
            None,
        );
        let after_metrics = projections.metrics();

        assert_eq!(
            flatten_projections(&before),
            flatten_projections(&after),
            "the semantic correction keeps this renderer's text geometry"
        );
        assert_eq!(
            after_metrics.stable_roots_rendered,
            before_metrics.stable_roots_rendered + 1
        );
        assert_eq!(
            after_metrics.stable_roots_reused,
            before_metrics.stable_roots_reused + 1
        );
    }

    #[test]
    fn full_replace_never_reuses_a_projection_from_the_previous_continuity() {
        let mut engine = StreamEngine::new();
        let mut reducer = TransitionReducer::new();
        let mut projections = ProjectionCache::default();
        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine.append("# old\n\n").expect("append must succeed"),
        );
        project_document(
            reducer.document().expect("append creates a document"),
            reducer.continuity_generation(),
            &mut projections,
            None,
        );

        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine.reset().expect("reset must succeed"),
        );
        apply_output_and_observe(
            &mut reducer,
            &mut projections,
            engine.append("# new\n\n").expect("append must succeed"),
        );
        let after = project_document(
            reducer.document().expect("append recreates a document"),
            reducer.continuity_generation(),
            &mut projections,
            None,
        );

        assert_eq!(
            line_texts(&after)
                .into_iter()
                .filter(|line| !line.is_empty())
                .collect::<Vec<_>>(),
            vec!["# new"]
        );
        assert_eq!(projections.metrics().stable_roots_rendered, 2);
        assert_eq!(projections.metrics().stable_roots_reused, 0);
        assert_eq!(projections.metrics().full_invalidations, 1);
    }

    #[test]
    fn cache_does_not_reuse_a_node_id_across_continuity_generations() {
        let mut syntax = SyntaxHighlighter::new().expect("fixture grammars compile");
        let version = NodeVersion::digest(b"same-version");
        let source = "fn main() {}";
        let first = key(7);
        let second = TransitionNodeKey {
            continuity_generation: ContinuityGeneration::new(1),
            ..first
        };

        syntax.render_code(first, &version, Some("rust"), source, true);
        syntax.render_code(second, &version, Some("rust"), source, true);

        assert_eq!(syntax.cache.len(), 2);
        assert!(syntax.cache.contains_key(&first));
        assert!(syntax.cache.contains_key(&second));
    }

    #[test]
    fn cache_evicts_the_least_recently_used_entry_not_the_smallest_key() {
        let mut syntax = SyntaxHighlighter::new().expect("fixture grammars compile");
        let version = NodeVersion::digest(b"lru-version");
        let source = " ".repeat(MAX_HIGHLIGHT_BYTES);
        let keys = [key(1), key(2), key(3), key(4), key(5)];

        for cache_key in keys.iter().take(4).copied() {
            syntax.insert_cache(
                cache_key,
                version.clone(),
                CodeLanguage::Rust,
                source.clone(),
                Vec::new(),
                0,
            );
        }
        assert!(
            syntax
                .cached_lines(keys[0], &version, CodeLanguage::Rust, &source)
                .is_some()
        );

        syntax.insert_cache(keys[4], version, CodeLanguage::Rust, source, Vec::new(), 0);

        assert!(syntax.cache.contains_key(&keys[0]));
        assert!(!syntax.cache.contains_key(&keys[1]));
        assert!(syntax.cache.contains_key(&keys[4]));
        assert_eq!(syntax.cache_bytes, MAX_HIGHLIGHT_CACHE_BYTES);
        assert_eq!(syntax.lru.len(), syntax.cache.len());
    }

    #[test]
    fn cache_entry_limit_bounds_many_tiny_code_blocks() {
        let mut syntax = SyntaxHighlighter::new().expect("fixture grammars compile");
        let version = NodeVersion::digest(b"tiny-entry-version");
        for node_id in 0..=MAX_HIGHLIGHT_CACHE_ENTRIES {
            syntax.insert_cache(
                key(node_id as u64),
                version.clone(),
                CodeLanguage::Rust,
                "x".to_string(),
                Vec::new(),
                0,
            );
        }

        assert_eq!(syntax.cache.len(), MAX_HIGHLIGHT_CACHE_ENTRIES);
        assert_eq!(syntax.lru.len(), MAX_HIGHLIGHT_CACHE_ENTRIES);
        assert!(!syntax.cache.contains_key(&key(0)));
        assert!(
            syntax
                .cache
                .contains_key(&key(MAX_HIGHLIGHT_CACHE_ENTRIES as u64))
        );
    }
}
