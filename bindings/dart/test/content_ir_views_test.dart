import 'dart:convert';
import 'dart:typed_data';

import 'package:mdstream/src/errors.dart';
import 'package:mdstream/src/protocol.dart';
import 'package:mdstream/src/views.dart';
import 'package:test/test.dart';

import 'support/fixtures.dart';

const schema = 'mdstream.bindings/0.4';

void main() {
  final fixture = loadFixture('conformance/bindings/content-ir.json');

  test('decodes every Rust ContentKind and SemanticResourceKind variant', () {
    final contents = list(fixture['content_kinds'], 'content_kinds')
        .map((value) => _decodeContent(record(value, 'content kind')))
        .toList(growable: false);
    final resources =
        list(fixture['semantic_resource_kinds'], 'semantic_resource_kinds')
            .map((value) => _decodeResource(record(value, 'resource kind')))
            .toList(growable: false);

    expect(contents.map(_describeContent), hasLength(28));
    expect(resources.map(_describeResource), hasLength(3));
    expect(
      contents.map((content) => content.kind).toSet(),
      list(
        fixture['content_kinds'],
        'content_kinds',
      ).map((value) => record(value, 'content kind')['kind']).toSet(),
    );
  });

  test('decodes every nested semantic and presentation metadata variant', () {
    for (final value in list(fixture['semantic_text'], 'semantic_text')) {
      final content = _decodeContent({
        'kind': 'text',
        'text': record(value, 'semantic text'),
      });
      expect(content, isA<TextContentView>());
    }
    for (final value in list(
      fixture['code_block_syntax'],
      'code_block_syntax',
    )) {
      final content = _decodeContent({
        'kind': 'code_block',
        'syntax': record(value, 'code syntax'),
        'info': null,
        'text': {'kind': 'source'},
      });
      expect(content, isA<CodeBlockContentView>());
    }
    for (final style in list(fixture['link_styles'], 'link_styles')) {
      final content = _decodeContent({
        'kind': 'link',
        'target': null,
        'reference_label': null,
        'style': style,
      });
      expect(content, isA<LinkContentView>());
    }
    for (final style in list(
      fixture['block_quote_kinds'],
      'block_quote_kinds',
    )) {
      expect(
        _decodeContent({'kind': 'block_quote', 'style': style}),
        isA<BlockQuoteContentView>(),
      );
    }
  });

  test('deep-freezes typed collection fields', () {
    final contents = list(
      fixture['content_kinds'],
      'content_kinds',
    ).map((value) => _decodeContent(record(value, 'content kind')));
    final table = contents.whereType<TableContentView>().single;
    final custom = contents.whereType<CustomContentView>().single;

    expect(
      () => table.alignments.add(TableAlignment.left),
      throwsUnsupportedError,
    );
    expect(() => custom.attributes['role'] = 'changed', throwsUnsupportedError);
  });

  test('rejects unknown, malformed, and variant-incompatible content', () {
    final malformed = <Map<String, Object?>>[
      {'kind': 'unknown'},
      {'kind': 'heading'},
      {'kind': 'paragraph', 'level': 1},
      {
        'kind': 'link',
        'target': null,
        'reference_label': null,
        'style': 'invalid',
      },
      {
        'kind': 'custom',
        'namespace': 'app',
        'name': 'panel',
        'opaque': true,
        'attributes': {'invalid': 1},
      },
    ];

    for (final content in malformed) {
      expect(
        () => _decodeContent(content),
        throwsA(
          isA<MdstreamException>().having(
            (error) => error.detailCode,
            'detailCode',
            'bindings.invalid_payload',
          ),
        ),
      );
    }
  });

  test('enforces u64 counters and u128 content IDs on native output', () {
    expect(
      _decodeNode({
        'kind': 'paragraph',
      }, id: '340282366920938463463374607431768211455').node.id,
      '340282366920938463463374607431768211455',
    );
    expect(
      () =>
          _decodeNode({'kind': 'paragraph'}, sourceEnd: '18446744073709551616'),
      throwsA(isA<MdstreamException>()),
    );
    expect(
      () => _decodeNode({
        'kind': 'paragraph',
      }, id: '340282366920938463463374607431768211456'),
      throwsA(isA<MdstreamException>()),
    );
  });
}

ContentKindView _decodeContent(Map<String, Object?> content) =>
    _decodeNode(content).node.content;

NodeView _decodeNode(
  Map<String, Object?> content, {
  String id = '7',
  String sourceEnd = '4',
}) =>
    decodeBindingView(
          BindingPayloadKind.nodeView,
          _bytes({
            'schema': schema,
            'kind': 'node_view',
            'node': {
              'id': id,
              'version': 'sha256:node',
              'stability': 'stable',
              'source': {'start': '0', 'end': sourceEnd},
              'body': {'start': '0', 'end': '4'},
              'children': {
                'version': 'sha256:children',
                'children': <Object?>[],
              },
              'content': content,
            },
            'body_text': 'body',
            'processor_input_version': 'sha256:processor-input',
          }),
          schema,
        )
        as NodeView;

SemanticResourceKindView _decodeResource(Map<String, Object?> content) =>
    (decodeBindingView(
              BindingPayloadKind.resourceView,
              _bytes({
                'schema': schema,
                'kind': 'resource_view',
                'resource': {
                  'id': '9',
                  'version': 'sha256:resource',
                  'content': content,
                },
              }),
              schema,
            )
            as ResourceView)
        .resource
        .content;

Uint8List _bytes(Map<String, Object?> value) =>
    Uint8List.fromList(utf8.encode(jsonEncode(value)));

String _describeContent(ContentKindView content) => switch (content) {
  ParagraphContentView() ||
  EmphasisContentView() ||
  StrongContentView() ||
  StrikethroughContentView() ||
  ThematicBreakContentView() ||
  TableHeadContentView() ||
  TableBodyContentView() ||
  TableRowContentView() ||
  SoftBreakContentView() ||
  HardBreakContentView() => content.kind,
  HeadingContentView(:final level) => 'heading:$level',
  TextContentView(:final text) => 'text:${text.kind}',
  LinkContentView(:final style, :final referenceLabel, :final target) =>
    'link:$style:${referenceLabel ?? target?.id ?? ''}',
  ImageContentView(:final style, :final alt) => 'image:$style:${alt.kind}',
  InlineCodeContentView(:final text) => 'inline:${text.kind}',
  CodeBlockContentView(:final syntax, :final info, :final text) =>
    'code:${syntax.kind}:${info ?? ''}:${text.kind}',
  ListContentView(:final ordered, :final start, :final tight) =>
    'list:$ordered:${start ?? ''}:$tight',
  ListItemContentView(:final checked) => 'item:${checked ?? ''}',
  BlockQuoteContentView(:final style) => 'quote:$style',
  TableContentView(:final alignments) => 'table:${alignments.join(',')}',
  TableCellContentView(:final column) => 'cell:$column',
  HtmlContentView(:final block, :final text) => 'html:$block:${text.kind}',
  MathContentView(:final display, :final text) => 'math:$display:${text.kind}',
  FootnoteDefinitionContentView(:final label, :final target) =>
    'footnote-definition:$label:${target.id}',
  FootnoteReferenceContentView(:final label, :final target) =>
    'footnote-reference:$label:${target?.id ?? ''}',
  CitationDefinitionContentView(:final key, :final target) =>
    'citation-definition:$key:${target.id}',
  CitationReferenceContentView(:final key, :final target) =>
    'citation-reference:$key:${target?.id ?? ''}',
  CustomContentView(:final namespace, :final name, :final opaque) =>
    'custom:$namespace:$name:$opaque',
};

String _describeResource(SemanticResourceKindView content) => switch (content) {
  LinkResourceContentView(:final destination, :final title) =>
    'link:$destination:${title ?? ''}',
  FootnoteResourceContentView(:final label) => 'footnote:$label',
  CitationResourceContentView(
    :final protocol,
    :final key,
    :final destination,
    :final title,
  ) =>
    'citation:$protocol:$key:$destination:${title ?? ''}',
};
