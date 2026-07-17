// ignore_for_file: public_member_api_docs

import 'package:flutter/material.dart';
import 'package:mdstream_flutter/mdstream_flutter.dart';

void main() {
  final runtime = MdstreamFlutterRuntime.open();
  runApp(MdstreamExample(version: runtime.packageVersion));
}

class MdstreamExample extends StatelessWidget {
  const MdstreamExample({required this.version, super.key});

  final String version;

  @override
  Widget build(BuildContext context) => MaterialApp(
    home: Scaffold(
      appBar: AppBar(title: const Text('mdstream')),
      body: Center(child: Text('Native runtime $version')),
    ),
  );
}
