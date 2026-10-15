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
  func testBundledHelpersAreSeparateFromAppExecutable() throws {
    let bundle = Bundle.main
    let app = try XCTUnwrap(bundle.executableURL)
    let helpers = bundle.bundleURL.appendingPathComponent("Contents/Helpers")
    for name in ["releash", "releashd"] {
      let helper = helpers.appendingPathComponent(name)
      XCTAssertTrue(FileManager.default.isExecutableFile(atPath: helper.path))
      XCTAssertNotEqual(helper.resolvingSymlinksInPath(), app.resolvingSymlinksInPath())
    }
    XCTAssertFalse(
      FileManager.default.fileExists(
        atPath: bundle.bundleURL.appendingPathComponent("Contents/MacOS/releashd").path))
    XCTAssertNotEqual(
      try Data(contentsOf: helpers.appendingPathComponent("releash")),
      try Data(contentsOf: app))
  }

  func testStartupStartsMissingServerAndReadsDiscoveryFromCLI() async throws {
    let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: file) }
    try Data(
      #"{"port":12345,"token":"operator","daemon_id":"daemon","pid":1,"process_started_at":"1"}"#
        .utf8
    ).write(to: file)
    let calls = Calls()
    let cli = CLI(
      executable: URL(fileURLWithPath: "/bundled/releash"),
      run: { _, args in
        let count = await calls.record(args)
        if args == ["server", "start", "--json"] { return Data() }
        return try JSONSerialization.data(withJSONObject: [
          "running": count > 1, "compatibility": "compatible", "discovery_file": file.path,
        ])
      })
    let discovery = try await cli.discover(startIfMissing: true)
    XCTAssertEqual(discovery.token, "operator")
    let recorded = await calls.values
    XCTAssertEqual(
      recorded, [["status", "--json"], ["server", "start", "--json"], ["status", "--json"]])
  }
}
