import 'dart:io';

import 'package:mdstream/mdstream.dart' as public_api;
import 'package:test/test.dart';
import 'package:yaml/yaml.dart';

void main() {
  test('standalone package stays framework and parser neutral', () {
    final root = Directory.current;
    final sources = Directory('${root.path}${Platform.pathSeparator}lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where((file) => file.path.endsWith('.dart'))
        .map((file) => file.readAsStringSync())
        .join('\n');

    for (final forbidden in <String>[
      'package:flutter',
      'package:react',
      'merman',
      'pulldown',
      'streamdown',
      'incremark',
      'insert_node',
      'replace_node',
      'remove_node',
      'finish_document',
      'class ChangeSet',
      'class ProjectionOp',
    ]) {
      expect(sources.toLowerCase(), isNot(contains(forbidden.toLowerCase())));
    }

    final entrypoint = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}mdstream.dart',
    ).readAsStringSync();
    expect(entrypoint, isNot(contains("export 'src/ffi.dart'")));
    expect(entrypoint, matches(RegExp(r"export 'src/protocol\.dart'\s+show")));
    expect(
      entrypoint,
      isNot(matches(RegExp(r"export 'src/protocol\.dart'\s+hide"))),
    );
    expect(entrypoint, isNot(contains('LosslessInputBatcher')));
    expect(entrypoint, isNot(contains('runBatchCandidateForTesting')));
    expect(entrypoint, contains('BatchPendingInput'));

    final batching = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}src${Platform.pathSeparator}batching.dart',
    ).readAsStringSync();
    expect(batching, isNot(contains('BatchCandidatePolicy')));
    expect(batching, isNot(contains('runBatchCandidateForTesting')));
    expect(batching, isNot(contains('_applyJoinedForEvaluation')));
    expect(batching, isNot(contains('_joinedFirstForEvaluation')));
    expect(batching, isNot(contains('_retryAtOriginalBoundaries')));

    final pubspec =
        loadYaml(
              File(
                '${root.path}${Platform.pathSeparator}pubspec.yaml',
              ).readAsStringSync(),
            )
            as YamlMap;
    expect((pubspec['dependencies'] as YamlMap).keys, <Object?>['ffi']);
    expect(pubspec.containsKey('flutter'), isFalse);
  });

  test('public API uses documented domain types and valid state unions', () {
    final root = Directory.current;
    final libraryFiles = Directory('${root.path}${Platform.pathSeparator}lib')
        .listSync(recursive: true)
        .whereType<File>()
        .where((file) => file.path.endsWith('.dart'))
        .toList(growable: false);
    final sources = libraryFiles
        .map((file) => file.readAsStringSync())
        .join('\n');

    expect(sources, isNot(contains('ignore_for_file: public_member_api_docs')));
    expect(
      sources,
      isNot(
        matches(
          RegExp(
            r'typedef\s+(DecimalCounter|Epoch|Sequence|SourceCursor|'
            r'RequestGeneration|ContinuityGeneration|NodeId|ResourceId|'
            r'ChangeId|NodeVersion|ResourceVersion|StructureVersion|'
            r'ProcessorInputVersion)\s*=\s*String',
          ),
        ),
      ),
    );

    final views = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}src${Platform.pathSeparator}views.dart',
    ).readAsStringSync();
    expect(
      views,
      isNot(contains('final String kind;\n  final CoordinateView?')),
    );
    expect(
      views,
      isNot(
        contains(
          'final CoordinateView? lastGood;\n  final RecoveryReasonView?',
        ),
      ),
    );
  });

  test('scheduler limits use one host value type around the ABI struct', () {
    final root = Directory.current;
    final options = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}src${Platform.pathSeparator}options.dart',
    ).readAsStringSync();
    final ffi = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}src${Platform.pathSeparator}ffi.dart',
    ).readAsStringSync();
    final reducer = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}src${Platform.pathSeparator}reducer_handle.dart',
    ).readAsStringSync();
    final entrypoint = File(
      '${root.path}${Platform.pathSeparator}lib'
      '${Platform.pathSeparator}mdstream.dart',
    ).readAsStringSync();

    expect(options, contains('final class MdstreamProcessorSchedulerLimits'));
    expect(
      ffi,
      contains(
        'final class _MdstreamProcessorSchedulerLimits extends ffi.Struct',
      ),
    );
    expect(ffi, isNot(contains('NativeProcessorSchedulerLimits')));
    expect(
      ffi,
      contains('processorSchedulerLimits = MdstreamProcessorSchedulerLimits('),
    );
    expect(
      reducer,
      contains('processorSchedulerLimits = _handle.processorSchedulerLimits'),
    );
    expect(
      reducer,
      isNot(contains('maxInFlightJobs: _handle.processorSchedulerLimits')),
    );
    expect(entrypoint, contains("export 'src/options.dart';"));
    final reducerExport = RegExp(
      r"export 'src/reducer_handle\.dart'\s+show[\s\S]*?;",
    ).firstMatch(entrypoint)?.group(0);
    expect(reducerExport, isNotNull);
    expect(reducerExport, isNot(contains('MdstreamProcessorSchedulerLimits')));
  });

  test('scheduler limits are available from the public entrypoint', () {
    const limits = public_api.MdstreamProcessorSchedulerLimits(
      maxInFlightJobs: 3,
      maxQueuedCandidates: 7,
    );

    expect(limits.maxInFlightJobs, 3);
    expect(limits.maxQueuedCandidates, 7);
  });

  test('NodeId rejects strings and other identifier domains', () async {
    final fixture = File(
      '${Directory.current.path}${Platform.pathSeparator}test'
      '${Platform.pathSeparator}.invalid_domain_ids.dart',
    );
    fixture.writeAsStringSync('''
import 'package:mdstream/mdstream.dart';

void acceptsNode(NodeId value) {}

void invalidAssignments() {
  acceptsNode('1');
  final resource = ResourceId.parse('1');
  acceptsNode(resource);
}
''');
    try {
      final result = await Process.run(Platform.resolvedExecutable, <String>[
        'analyze',
        '--format=machine',
        fixture.path,
      ], workingDirectory: Directory.current.path);
      final diagnostics = '${result.stdout}\n${result.stderr}';
      expect(result.exitCode, isNot(0), reason: diagnostics);
      expect(
        RegExp('ARGUMENT_TYPE_NOT_ASSIGNABLE').allMatches(diagnostics).length,
        greaterThanOrEqualTo(2),
        reason: diagnostics,
      );
    } finally {
      if (fixture.existsSync()) {
        fixture.deleteSync();
      }
    }
  });

  test(
    'package-internal protocol helpers stay out of the entrypoint',
    () async {
      final fixture = File(
        '${Directory.current.path}${Platform.pathSeparator}test'
        '${Platform.pathSeparator}.invalid_protocol_helper.dart',
      );
      fixture.writeAsStringSync('''
import 'package:mdstream/mdstream.dart';

void invalidAccess() {
  decimalCounterFromTrustedInt(1);
}
''');
      try {
        final result = await Process.run(Platform.resolvedExecutable, <String>[
          'analyze',
          '--format=machine',
          fixture.path,
        ], workingDirectory: Directory.current.path);
        final diagnostics = '${result.stdout}\n${result.stderr}';
        expect(result.exitCode, isNot(0), reason: diagnostics);
        expect(
          diagnostics,
          contains('UNDEFINED_FUNCTION'),
          reason: diagnostics,
        );
      } finally {
        if (fixture.existsSync()) {
          fixture.deleteSync();
        }
      }
    },
  );

  test('compiler migration docs keep the typed Dart API', () async {
    final packageRoot = Directory.current;
    final repositoryRoot = packageRoot.parent.parent;
    final usage = File(
      '${repositoryRoot.path}${Platform.pathSeparator}docs'
      '${Platform.pathSeparator}USAGE.md',
    ).readAsStringSync();
    final marker = RegExp(
      r'Dart exposes them as typed\s+'
      r'`MdstreamCompilerLimits` parameters:',
    ).firstMatch(usage);
    expect(marker, isNotNull);
    expect(
      usage,
      isNot(
        matches(
          RegExp(
            r'Dart uses[\s\S]*snake-case keys[\s\S]*compiler map',
            caseSensitive: false,
          ),
        ),
      ),
    );

    const openingFence = '```dart\n';
    final markerEnd = marker!.end;
    final fenceOffset = usage.indexOf(openingFence, markerEnd);
    expect(fenceOffset, greaterThan(markerEnd));
    final sourceOffset = fenceOffset + openingFence.length;
    final closingFence = usage.indexOf('\n```', sourceOffset);
    expect(closingFence, greaterThan(sourceOffset));
    final documentedSource = usage.substring(sourceOffset, closingFence);

    final validFixture = File(
      '${packageRoot.path}${Platform.pathSeparator}test'
      '${Platform.pathSeparator}.usage_compiler_limits.dart',
    );
    final invalidFixture = File(
      '${packageRoot.path}${Platform.pathSeparator}test'
      '${Platform.pathSeparator}.removed_compiler_map.dart',
    );
    final invalidProtocolFixture = File(
      '${packageRoot.path}${Platform.pathSeparator}test'
      '${Platform.pathSeparator}.removed_protocol_compiler_limits.dart',
    );
    validFixture.writeAsStringSync(documentedSource);
    invalidFixture.writeAsStringSync('''
import 'package:mdstream/mdstream.dart';

void invalidMapApi() {
  MdstreamSessionOptions(
    compiler: <String, String>{
      'max_markdown_events': '1',
    },
  );
}
''');
    invalidProtocolFixture.writeAsStringSync('''
import 'package:mdstream/mdstream.dart';

void invalidProtocolLimits() {
  MdstreamProtocolLimits(
    maxDefinitions: '1',
    maxDefinitionEdges: '1',
    maxDefinitionMetadataBytes: '1',
  );
}
''');
    try {
      final validResult = await Process.run(
        Platform.resolvedExecutable,
        <String>['analyze', '--format=machine', validFixture.path],
        workingDirectory: packageRoot.path,
      );
      final validDiagnostics = '${validResult.stdout}\n${validResult.stderr}';
      expect(validResult.exitCode, 0, reason: validDiagnostics);

      final invalidResult = await Process.run(
        Platform.resolvedExecutable,
        <String>['analyze', '--format=machine', invalidFixture.path],
        workingDirectory: packageRoot.path,
      );
      final invalidDiagnostics =
          '${invalidResult.stdout}\n${invalidResult.stderr}';
      expect(invalidResult.exitCode, isNot(0), reason: invalidDiagnostics);
      expect(
        invalidDiagnostics,
        contains('ARGUMENT_TYPE_NOT_ASSIGNABLE'),
        reason: invalidDiagnostics,
      );

      final invalidProtocolResult = await Process.run(
        Platform.resolvedExecutable,
        <String>['analyze', '--format=machine', invalidProtocolFixture.path],
        workingDirectory: packageRoot.path,
      );
      final invalidProtocolDiagnostics =
          '${invalidProtocolResult.stdout}\n${invalidProtocolResult.stderr}';
      expect(
        invalidProtocolResult.exitCode,
        isNot(0),
        reason: invalidProtocolDiagnostics,
      );
      for (final field in <String>[
        'maxDefinitions',
        'maxDefinitionEdges',
        'maxDefinitionMetadataBytes',
      ]) {
        expect(
          invalidProtocolDiagnostics,
          contains(field),
          reason: invalidProtocolDiagnostics,
        );
      }
      expect(
        invalidProtocolDiagnostics,
        contains('UNDEFINED_NAMED_PARAMETER'),
        reason: invalidProtocolDiagnostics,
      );
    } finally {
      if (validFixture.existsSync()) {
        validFixture.deleteSync();
      }
      if (invalidFixture.existsSync()) {
        invalidFixture.deleteSync();
      }
      if (invalidProtocolFixture.existsSync()) {
        invalidProtocolFixture.deleteSync();
      }
    }
  });
}
