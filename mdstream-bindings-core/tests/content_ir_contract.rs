use std::collections::BTreeSet;

use mdstream_protocol::{
    BlockQuoteKind, CodeBlockSyntax, CodeFenceMarker, ContentKind, LinkStyle, SemanticResourceKind,
    SemanticText, TableAlignment,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct ContentIrFixture {
    schema: String,
    semantic_text: Vec<SemanticText>,
    code_block_syntax: Vec<CodeBlockSyntax>,
    link_styles: Vec<LinkStyle>,
    block_quote_kinds: Vec<BlockQuoteKind>,
    table_alignments: Vec<TableAlignment>,
    content_kinds: Vec<ContentKind>,
    semantic_resource_kinds: Vec<SemanticResourceKind>,
}

#[test]
fn binding_fixture_exhaustively_tracks_the_rust_content_ir_vocabulary() {
    let fixture: ContentIrFixture =
        serde_json::from_str(include_str!("../../conformance/bindings/content-ir.json")).unwrap();

    assert_eq!(fixture.schema, "mdstream.binding-content-ir-fixture/1");
    assert_eq!(
        fixture
            .content_kinds
            .iter()
            .map(content_kind_name)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "paragraph",
            "heading",
            "text",
            "emphasis",
            "strong",
            "strikethrough",
            "link",
            "image",
            "inline_code",
            "code_block",
            "list",
            "list_item",
            "block_quote",
            "thematic_break",
            "table",
            "table_head",
            "table_body",
            "table_row",
            "table_cell",
            "html",
            "math",
            "footnote_definition",
            "footnote_reference",
            "citation_definition",
            "citation_reference",
            "soft_break",
            "hard_break",
            "custom",
        ])
    );
    assert_eq!(
        fixture
            .semantic_resource_kinds
            .iter()
            .map(resource_kind_name)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["link", "footnote", "citation"])
    );
    assert!(matches!(
        fixture.semantic_text.as_slice(),
        [SemanticText::Source {}, SemanticText::Normalized { .. }]
    ));
    assert!(matches!(
        fixture.code_block_syntax[0],
        CodeBlockSyntax::Indented
    ));
    assert!(matches!(
        fixture.code_block_syntax[1],
        CodeBlockSyntax::Fenced {
            marker: CodeFenceMarker::Backtick,
            length: 3
        }
    ));
    assert!(matches!(
        fixture.code_block_syntax[2],
        CodeBlockSyntax::Fenced {
            marker: CodeFenceMarker::Tilde,
            length: 4
        }
    ));
    assert_eq!(fixture.link_styles.len(), 9);
    assert_eq!(fixture.block_quote_kinds.len(), 6);
    assert_eq!(fixture.table_alignments.len(), 4);
}

fn content_kind_name(content: &ContentKind) -> &'static str {
    match content {
        ContentKind::Paragraph {} => "paragraph",
        ContentKind::Heading { .. } => "heading",
        ContentKind::Text { .. } => "text",
        ContentKind::Emphasis {} => "emphasis",
        ContentKind::Strong {} => "strong",
        ContentKind::Strikethrough {} => "strikethrough",
        ContentKind::Link { .. } => "link",
        ContentKind::Image { .. } => "image",
        ContentKind::InlineCode { .. } => "inline_code",
        ContentKind::CodeBlock { .. } => "code_block",
        ContentKind::List { .. } => "list",
        ContentKind::ListItem { .. } => "list_item",
        ContentKind::BlockQuote { .. } => "block_quote",
        ContentKind::ThematicBreak {} => "thematic_break",
        ContentKind::Table { .. } => "table",
        ContentKind::TableHead {} => "table_head",
        ContentKind::TableBody {} => "table_body",
        ContentKind::TableRow {} => "table_row",
        ContentKind::TableCell { .. } => "table_cell",
        ContentKind::Html { .. } => "html",
        ContentKind::Math { .. } => "math",
        ContentKind::FootnoteDefinition { .. } => "footnote_definition",
        ContentKind::FootnoteReference { .. } => "footnote_reference",
        ContentKind::CitationDefinition { .. } => "citation_definition",
        ContentKind::CitationReference { .. } => "citation_reference",
        ContentKind::SoftBreak {} => "soft_break",
        ContentKind::HardBreak {} => "hard_break",
        ContentKind::Custom { .. } => "custom",
    }
}

fn resource_kind_name(content: &SemanticResourceKind) -> &'static str {
    match content {
        SemanticResourceKind::Link { .. } => "link",
        SemanticResourceKind::Footnote { .. } => "footnote",
        SemanticResourceKind::Citation { .. } => "citation",
    }
}
