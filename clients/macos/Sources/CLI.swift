import Foundation
import SwiftProtobuf

struct CLIStatus: Decodable, Sendable {
  let running: Bool
  let compatibility: String?
  let guidance: String?
  let discoveryFile: String
  private enum CodingKeys: String, CodingKey {
    case running
    case compatibility
    case guidance
    case discoveryFile = "discovery_file"
  }
}

struct CLIFailure: Error, LocalizedError, Sendable {
  let message: String
  var errorDescription: String? { message }
}

struct CLI: Sendable {
  let executable: URL
  var run: @Sendable (URL, [String]) async throws -> Data = { executable, arguments in
    try await Task.detached {
      let process = Process()
      process.executableURL = executable
      process.arguments = arguments
      let stdout = Pipe()
      let stderr = Pipe()
      process.standardOutput = stdout
      process.standardError = stderr
      try process.run()
      let data = stdout.fileHandleForReading.readDataToEndOfFile()
      let error = stderr.fileHandleForReading.readDataToEndOfFile()
      process.waitUntilExit()
      guard process.terminationStatus == 0 else {
        throw CLIFailure(message: String(decoding: error.isEmpty ? data : error, as: UTF8.self))
      }
      return data
    }.value
  }

  func discover(startIfMissing: Bool) async throws -> Releash_Client_V1_LocalApiDiscovery {
    var status = try JSONDecoder().decode(
      CLIStatus.self, from: await run(executable, ["status", "--json"]))
    if !status.running && startIfMissing {
      _ = try await run(executable, ["server", "start"])
      status = try JSONDecoder().decode(
        CLIStatus.self, from: await run(executable, ["status", "--json"]))
    }
    guard status.running else {
      throw CLIFailure(
        message: status.guidance ?? "Server not found. Run releash server start, then retry.")
    }
    guard status.compatibility == "compatible" else {
      throw CLIFailure(
        message:
          "\(status.compatibility ?? "Compatibility unavailable")\n\(status.guidance ?? "Run releash status --json.")"
      )
    }
    let data = try Data(contentsOf: URL(fileURLWithPath: status.discoveryFile))
    return try Releash_Client_V1_LocalApiDiscovery(jsonUTF8Data: data)
  }
}
