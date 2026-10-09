import Foundation
import SwiftProtobuf

struct CLIStatus: Decodable, Sendable {
  let running: Bool
  let compatibility: String?
  let guidance: String?
  let startupGuidance: String?
  let discoveryFile: String
  private enum CodingKeys: String, CodingKey {
    case running
    case compatibility
    case guidance
    case startupGuidance = "startup_guidance"
    case discoveryFile = "discovery_file"
  }
}

struct CLIFailure: Error, LocalizedError, Sendable {
  let message: String
  var errorDescription: String? { message }
}

private struct CLIErrorOutput: Decodable {
  struct Failure: Decodable {
    let message: String
    let guidance: String?
  }
  let error: Failure
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
    var category = "サーバを発見できません"
    do {
      var status = try JSONDecoder().decode(
        CLIStatus.self, from: await run(executable, ["status", "--json"]))
      if !status.running && startIfMissing {
        category = "サーバを起動できません"
        _ = try await run(executable, ["server", "start"])
        category = "サーバを発見できません"
        status = try JSONDecoder().decode(
          CLIStatus.self, from: await run(executable, ["status", "--json"]))
      }
      guard status.running else {
        throw CLIFailure(message: status.startupGuidance ?? "")
      }
      guard status.compatibility == "compatible" else {
        category = "サーバと互換性がありません"
        throw CLIFailure(
          message: [status.compatibility, status.guidance].compactMap { $0 }.joined(separator: "\n"))
      }
      let data = try Data(contentsOf: URL(fileURLWithPath: status.discoveryFile))
      return try Releash_Client_V1_LocalApiDiscovery(jsonUTF8Data: data)
    } catch {
      let message = error.localizedDescription
      let output = try? JSONDecoder().decode(CLIErrorOutput.self, from: Data(message.utf8))
      let detail = output.map {
        [$0.error.message, $0.error.guidance].compactMap { $0 }.joined(separator: "\n")
      } ?? message
      throw CLIFailure(message: "\(category)\n\(detail)")
    }
  }
}
