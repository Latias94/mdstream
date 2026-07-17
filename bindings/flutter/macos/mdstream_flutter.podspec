Pod::Spec.new do |s|
  s.name                  = 'mdstream_flutter'
  s.version               = '0.4.0'
  s.summary               = 'Turnkey Flutter bindings for the mdstream content engine.'
  s.description           = <<-DESC
    Bundles the mdstream Rust FFI runtime for framework-neutral streaming
    content state in Flutter applications.
  DESC
  s.homepage              = 'https://github.com/Latias94/mdstream'
  s.license               = { :type => 'MIT', :file => '../LICENSE' }
  s.author                = { 'mdstream contributors' => 'https://github.com/Latias94/mdstream' }
  s.source                = { :path => '.' }
  s.platform              = :osx, '11.0'
  s.dependency            'FlutterMacOS'
  s.vendored_frameworks   = 'MdstreamFFI.xcframework'
  s.swift_version         = '5.9'
  s.pod_target_xcconfig   = { 'DEFINES_MODULE' => 'YES' }
end
