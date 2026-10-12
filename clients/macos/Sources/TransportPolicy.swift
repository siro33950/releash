import Foundation
import SwiftProtobuf

struct TransportPolicy {
  let options: Google_Protobuf_ServiceOptions
  init(bundle: Bundle = .main) throws {
    guard let url = bundle.url(forResource: "client-descriptor", withExtension: "bin") else {
      throw CLIFailure(message: "Bundled protocol descriptor is missing")
    }
    let descriptors = try Google_Protobuf_FileDescriptorSet(
      serializedBytes: Data(contentsOf: url), extensions: Releash_Client_V1_ClientOptions_Extensions
    )
    guard
      let service = descriptors.file.flatMap(\.service).first(where: { $0.name == "ClientService" })
    else {
      throw CLIFailure(message: "ClientService descriptor is missing")
    }
    options = service.options
  }
  var timeout: TimeInterval { Double(options.Releash_Client_V1_defaultTimeoutMs) / 1000 }
  var silence: TimeInterval { Double(options.Releash_Client_V1_stateStreamSilenceMs) / 1000 }
  func backoff(attempt: Int) -> TimeInterval {
    let config = options.Releash_Client_V1_connectionBackoff
    let value = min(
      Double(config.maxBackoffMs),
      Double(config.initialBackoffMs) * pow(Double(config.multiplier), Double(attempt)))
    return value * Double.random(in: (1 - Double(config.jitter))...(1 + Double(config.jitter)))
      / 1000
  }
}
