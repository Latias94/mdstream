import 'dart:io';

/// A deterministic failure while preparing a generated Apple host.
class ConfigureHostException implements Exception {
  /// Creates a host-configuration failure with a user-facing [message].
  const ConfigureHostException(this.message);

  /// Explains how the generated host violates the example contract.
  final String message;

  @override
  String toString() => message;
}

const _targets =
    <String, ({String setting, String podPlatform, String minimum})>{
      'ios': (
        setting: 'IPHONEOS_DEPLOYMENT_TARGET',
        podPlatform: 'ios',
        minimum: '14.0',
      ),
      'macos': (
        setting: 'MACOSX_DEPLOYMENT_TARGET',
        podPlatform: 'osx',
        minimum: '11.0',
      ),
    };

/// Applies mdstream's minimum deployment target to a generated Apple host.
void configureHost({required Directory projectRoot, required String platform}) {
  final target = _targets[platform];
  if (target == null) {
    throw ConfigureHostException('unsupported Apple platform: $platform');
  }

  final platformRoot = Directory(
    '${projectRoot.path}${Platform.pathSeparator}$platform',
  );
  final project = File(
    '${platformRoot.path}${Platform.pathSeparator}Runner.xcodeproj'
    '${Platform.pathSeparator}project.pbxproj',
  );
  final podfile = File('${platformRoot.path}${Platform.pathSeparator}Podfile');
  if (!project.existsSync() || !podfile.existsSync()) {
    throw ConfigureHostException(
      'run flutter create --platforms $platform before configuring the host',
    );
  }

  final projectPattern = RegExp(
    '(${RegExp.escape(target.setting)}\\s*=\\s*)'
    r'[0-9]+(?:\.[0-9]+)*(;)',
  );
  final podfilePattern = RegExp(
    '^\\s*#?\\s*platform\\s+:'
    '${RegExp.escape(target.podPlatform)}'
    r'''\s*,\s*['"][^'"]+['"]\s*$''',
    multiLine: true,
  );
  final originalProject = project.readAsStringSync();
  final originalPodfile = podfile.readAsStringSync();
  if (!projectPattern.hasMatch(originalProject) ||
      !podfilePattern.hasMatch(originalPodfile)) {
    throw ConfigureHostException(
      'generated $platform host omitted deployment target metadata',
    );
  }

  final configuredProject = originalProject.replaceAllMapped(
    projectPattern,
    (match) => '${match.group(1)}${target.minimum}${match.group(2)}',
  );
  final configuredPodfile = originalPodfile.replaceAll(
    podfilePattern,
    "platform :${target.podPlatform}, '${target.minimum}'",
  );
  _replace(project, configuredProject);
  _replace(podfile, configuredPodfile);
}

void _replace(File destination, String contents) {
  final temporary = File('${destination.path}.mdstream.tmp');
  temporary.writeAsStringSync(contents, flush: true);
  temporary.renameSync(destination.path);
}
