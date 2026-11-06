import Network
import Sparkle
import XCTest

@testable import Releash

final class UpdateCandidateTests: XCTestCase {
  @MainActor func test_更新候補_チャンネルと現在版で選別し昇格で再提示しない() async throws {
    for (current, stable, nightly, acceptsNightly, expected) in [
      ("0.4.20", "0.4.20", "0.4.21", false, Optional<String>.none),
      ("0.4.20", "0.4.20", "0.4.21", true, "0.4.21"),
      ("0.4.21", "0.4.20", "0.4.21", false, nil),
      ("0.4.21", "0.4.21", "0.4.21", false, nil),
      ("0.4.21", "0.4.22", "0.4.21", false, "0.4.22"),
    ] {
      // Given
      let feed = try CandidateFeed(stable: stable, nightly: nightly)
      defer { feed.stop() }
      let port = try await feed.port()
      let directory = FileManager.default.temporaryDirectory.appendingPathComponent(
        UUID().uuidString)
      let contents = directory.appendingPathComponent("Candidate.app/Contents")
      try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
      defer { try? FileManager.default.removeItem(at: directory) }
      let identifier = "test.releash.candidate.\(UUID().uuidString)"
      let defaults = try XCTUnwrap(UserDefaults(suiteName: identifier))
      defer { defaults.removePersistentDomain(forName: identifier) }
      let info: [String: Any] = [
        "CFBundleIdentifier": identifier, "CFBundleName": "Candidate",
        "CFBundleVersion": current, "CFBundleShortVersionString": current,
        "SUFeedURL": "http://127.0.0.1:\(port)/appcast.xml",
        "SUEnableAutomaticChecks": false, "SUAutomaticallyUpdate": false,
        "SUPublicEDKey": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        "NSAppTransportSecurity": ["NSAllowsLocalNetworking": true],
      ]
      try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
        .write(to: contents.appendingPathComponent("Info.plist"))
      let bundle = try XCTUnwrap(Bundle(url: contents.deletingLastPathComponent()))
      let app = AppUpdater(defaults: defaults)
      app.acceptsNightly = acceptsNightly
      let delegate = CandidateDelegate(app: app)
      let driver = SPUStandardUserDriver(hostBundle: bundle, delegate: nil)
      let updater = SPUUpdater(
        hostBundle: bundle, applicationBundle: Bundle.main, userDriver: driver, delegate: delegate)
      try updater.start()
      // When
      updater.checkForUpdateInformation()
      XCTAssertFalse(updater.canCheckForUpdates)
      let deadline = Date().addingTimeInterval(15)
      while updater.sessionInProgress && Date() < deadline {
        try await Task.sleep(for: .milliseconds(20))
      }
      // Then
      XCTAssertFalse(updater.sessionInProgress)
      XCTAssertTrue(updater.canCheckForUpdates)
      XCTAssertTrue(delegate.receivedResult, "Sparkle did not report a candidate result")
      XCTAssertEqual(app.availableVersion, expected)
    }
  }
}

private final class CandidateFeed: @unchecked Sendable {
  private let listener: NWListener

  init(stable: String, nightly: String) throws {
    listener = try NWListener(using: .tcp, on: .any)
    let item = { (version: String, channel: String) in
      """
      <item><sparkle:version>\(version)</sparkle:version><sparkle:shortVersionString>\(version)</sparkle:shortVersionString>\(channel)<enclosure url="https://example.invalid/update.zip" length="10" type="application/octet-stream" sparkle:edSignature="\(String(repeating: "a", count: 86))=="/></item>
      """
    }
    let xml = """
      <?xml version="1.0"?><rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><title>Test</title>\(item(stable, ""))\(item(nightly, "<sparkle:channel>nightly</sparkle:channel>"))</channel></rss>
      """
    let response = Data(
      "HTTP/1.1 200 OK\r\nContent-Type: application/xml\r\nContent-Length: \(xml.utf8.count)\r\nConnection: close\r\n\r\n\(xml)"
        .utf8)
    listener.newConnectionHandler = { connection in
      connection.start(queue: .global())
      connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { _, _, _, _ in
        connection.send(
          content: response, completion: .contentProcessed { _ in connection.cancel() })
      }
    }
    listener.start(queue: .global())
  }

  func port() async throws -> UInt16 {
    let deadline = Date().addingTimeInterval(5)
    while Date() < deadline {
      if let port = listener.port, port != .any { return port.rawValue }
      try await Task.sleep(for: .milliseconds(20))
    }
    throw NSError(domain: "CandidateFeed", code: 1)
  }

  func stop() { listener.cancel() }
}

@MainActor private final class CandidateDelegate: NSObject, SPUUpdaterDelegate {
  let app: AppUpdater
  var receivedResult = false

  init(app: AppUpdater) { self.app = app }

  func allowedChannels(for updater: SPUUpdater) -> Set<String> {
    app.allowedChannels(for: updater)
  }

  func updater(_ updater: SPUUpdater, didFindValidUpdate item: SUAppcastItem) {
    receivedResult = true
    app.updater(updater, didFindValidUpdate: item)
  }

  func updaterDidNotFindUpdate(_ updater: SPUUpdater, error: any Error) {
    receivedResult = true
    app.updaterDidNotFindUpdate(updater, error: error)
  }
}
