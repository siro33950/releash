import CryptoKit
import Foundation
import Network
import XCTest

final class UpdateFixture: @unchecked Sendable {
  let fixture: Fixture
  let identifier = "test.releash.update.\(UUID().uuidString)"
  let application: URL
  private let listener: NWListener

  init(fixture: Fixture) throws {
    self.fixture = fixture
    application = fixture.directory.appendingPathComponent("installed/Releash.app")
    listener = try NWListener(using: .tcp, on: .any)
    let directory = fixture.directory
    listener.newConnectionHandler = { connection in
      connection.start(queue: .global())
      connection.receive(minimumIncompleteLength: 1, maximumLength: 65536) { data, _, _, _ in
        let path = String(decoding: data ?? Data(), as: UTF8.self).split(separator: " ").dropFirst()
          .first
        let archive = path == "/update.zip"
        let file = directory.appendingPathComponent(archive ? "update.zip" : "appcast.xml")
        if archive {
          try? Data().write(to: directory.appendingPathComponent("download-requested"))
        }
        DispatchQueue.global().asyncAfter(deadline: .now() + 1) {
          if !archive {
            try? Data().write(to: directory.appendingPathComponent("feed-requested"))
            let deadline = Date().addingTimeInterval(45)
            while FileManager.default.fileExists(
              atPath: directory.appendingPathComponent("hold-feed").path)
              && Date() < deadline
            {
              Thread.sleep(forTimeInterval: 0.1)
            }
          }
          if archive {
            let deadline = Date().addingTimeInterval(45)
            let gate = directory.appendingPathComponent("release-download").path
            while !FileManager.default.fileExists(atPath: gate) && Date() < deadline {
              Thread.sleep(forTimeInterval: 0.1)
            }
          }
          do {
            let body = try Data(contentsOf: file)
            var response = Data(
              "HTTP/1.1 200 OK\r\nContent-Length: \(body.count)\r\nContent-Type: \(archive ? "application/zip" : "application/xml")\r\nConnection: close\r\n\r\n"
                .utf8)
            response.append(body)
            connection.send(
              content: response, completion: .contentProcessed { _ in connection.cancel() })
          } catch {
            connection.cancel()
          }
        }
      }
    }
    listener.start(queue: .global())
  }

  func prepare(
    automatic: Bool? = nil, debug: Bool = false
  ) async throws {
    let deadline = Date().addingTimeInterval(5)
    while (listener.port == nil || listener.port == .any) && Date() < deadline {
      try await Task.sleep(for: .milliseconds(20))
    }
    let port = try XCTUnwrap(listener.port).rawValue
    guard port != 0 else { throw NSError(domain: "UpdateFixture", code: 1) }
    let key = Curve25519.Signing.PrivateKey()
    let source = fixture.root.appendingPathComponent(
      "clients/macos/.build/E2EDerivedData/Build/Products/Debug/Releash.app")
    let updated = fixture.directory.appendingPathComponent("new/Releash.app")
    let updatedCLI = fixture.root.appendingPathComponent(
      "clients/macos/.build/Helpers/Update/releash")
    let status = try XCTUnwrap(
      JSONSerialization.jsonObject(with: fixture.process(updatedCLI, ["status", "--json"]))
        as? [String: Any])
    let cliVersion = (status["client"] as? [String: Any])?["release"] as? String
    guard cliVersion == "0.4.21" else {
      throw NSError(
        domain: "UpdateFixture", code: 2,
        userInfo: [
          NSLocalizedDescriptionKey:
            "Updated CLI must be releash 0.4.21: \(cliVersion ?? "missing")"
        ])
    }
    for (destination, version) in [(application, "0.4.20"), (updated, "0.4.21")] {
      try fixture.process(URL(fileURLWithPath: "/usr/bin/ditto"), [source.path, destination.path])
      let plist = destination.appendingPathComponent("Contents/Info.plist")
      var info = try XCTUnwrap(
        PropertyListSerialization.propertyList(from: Data(contentsOf: plist), format: nil)
          as? [String: Any])
      info["CFBundleIdentifier"] = debug ? "com.releash.macos.dev" : identifier
      if debug { info["SUDefaultsDomain"] = identifier }
      info["CFBundleVersion"] = version
      info["CFBundleShortVersionString"] = version
      info["SUFeedURL"] = "http://127.0.0.1:\(port)/appcast.xml"
      info["SUPublicEDKey"] = key.publicKey.rawRepresentation.base64EncodedString()
      if let automatic {
        info["SUEnableAutomaticChecks"] = automatic
        info["SUAutomaticallyUpdate"] = automatic
      }
      info["LSEnvironment"] = fixture.environment
      try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0).write(
        to: plist)
      if version == "0.4.21" {
        let cli = destination.appendingPathComponent("Contents/Helpers/releash")
        try FileManager.default.removeItem(at: cli)
        try FileManager.default.copyItem(at: updatedCLI, to: cli)
      }
      try fixture.process(
        URL(fileURLWithPath: "/usr/bin/codesign"),
        ["--force", "--deep", "--sign", "-", destination.path])
    }
    let archive = fixture.directory.appendingPathComponent("update.zip")
    try fixture.process(
      URL(fileURLWithPath: "/usr/bin/ditto"),
      ["-c", "-k", "--sequesterRsrc", "--keepParent", updated.path, archive.path])
    let bytes = try Data(contentsOf: archive)
    let signature = try key.signature(for: bytes).base64EncodedString()
    let enclosure = """
      <enclosure url="http://127.0.0.1:\(port)/update.zip" length="\(bytes.count)" type="application/octet-stream" sparkle:edSignature="\(signature)"/>
      """
    let feed = """
      <?xml version="1.0"?><rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle"><channel><title>Update test</title><item><title>0.4.21</title><sparkle:version>0.4.21</sparkle:version><sparkle:shortVersionString>0.4.21</sparkle:shortVersionString>\(enclosure)</item></channel></rss>
      """
    try feed.write(
      to: fixture.directory.appendingPathComponent("appcast.xml"), atomically: true, encoding: .utf8
    )
  }

  func installedVersion() throws -> String? {
    let info = try PropertyListSerialization.propertyList(
      from: Data(contentsOf: application.appendingPathComponent("Contents/Info.plist")), format: nil
    )
    return (info as? [String: Any])?["CFBundleVersion"] as? String
  }

  func cleanUp() {
    listener.cancel()
    UserDefaults(suiteName: identifier)?.removePersistentDomain(forName: identifier)
  }
}
