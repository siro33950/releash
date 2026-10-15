import XCTest

@testable import Releash

private actor Calls {
  var values: [[String]] = []
  func record(_ value: [String]) -> Int {
    values.append(value)
    return values.count
  }
}

final class CLITests: XCTestCase {

  func testRecoveryDoesNotStartServer() async {
    let calls = Calls()
    let cli = CLI(
      executable: URL(fileURLWithPath: "/bundled/releash"),
      run: { _, args in
        _ = await calls.record(args)
        return Data(
          #"{"running":false,"discovery_file":"/missing","startup_guidance":"CLI guidance"}"#.utf8)
      })
    do {
      _ = try await cli.discover(startIfMissing: false)
      XCTFail("Expected failure")
    } catch { XCTAssertTrue(error.localizedDescription.contains("CLI guidance")) }
    let recorded = await calls.values
    XCTAssertEqual(recorded, [["status", "--json"]])
  }
  func testIncompatibleStatusPreservesCLIGuidance() async {
    let cli = CLI(
      executable: URL(fileURLWithPath: "/bundled/releash"),
      run: { _, _ in
        Data(
          #"{"running":true,"compatibility":"server_older","guidance":"upgrade server","discovery_file":"/missing"}"#
            .utf8)
      })
    do {
      _ = try await cli.discover(startIfMissing: true)
      XCTFail("Expected failure")
    } catch {
      XCTAssertTrue(error.localizedDescription.contains("server_older"))
      XCTAssertTrue(error.localizedDescription.contains("upgrade server"))
    }
  }
  func testServerOperationsUseBundledCLIAndReleaseResultIsDecoded() async throws {
    let calls = Calls()
    let cli = CLI(
      executable: URL(fileURLWithPath: "/bundled/releash"),
      run: { executable, args in
        XCTAssertEqual(executable.path, "/bundled/releash")
        _ = await calls.record(args)
        return Data(
          #"{"running":true,"compatibility":"compatible","release_compatibility":"server_older","discovery_file":"/missing"}"#
            .utf8)
      })
    try await cli.stopServer()
    try await cli.restartServer()
    let status = try JSONDecoder().decode(
      CLIStatus.self, from: await cli.run(cli.executable, ["status", "--json"]))
    XCTAssertEqual(status.compatibility, "compatible")
    XCTAssertEqual(status.releaseCompatibility, "server_older")
    let recorded = await calls.values
    XCTAssertEqual(recorded, [["server", "stop"], ["server", "restart"], ["status", "--json"]])
  }

}
