import Sparkle
import XCTest

final class AppUpdaterDefaultsTests: XCTestCase {
  @MainActor func test_初回起動_製品の設定から自動確認とダウンロードが有効になる() throws {
    // Given
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let contents = directory.appendingPathComponent("Releash.app/Contents")
    try FileManager.default.createDirectory(at: contents, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let identifier = "test.releash.defaults.\(UUID().uuidString)"
    let defaults = try XCTUnwrap(UserDefaults(suiteName: identifier))
    defer { defaults.removePersistentDomain(forName: identifier) }
    var info = try XCTUnwrap(Bundle.main.infoDictionary)
    info["CFBundleIdentifier"] = identifier
    try PropertyListSerialization.data(fromPropertyList: info, format: .xml, options: 0)
      .write(to: contents.appendingPathComponent("Info.plist"))
    let bundle = try XCTUnwrap(Bundle(url: contents.deletingLastPathComponent()))
    XCTAssertNil(defaults.object(forKey: "SUEnableAutomaticChecks"))
    XCTAssertNil(defaults.object(forKey: "SUAutomaticallyUpdate"))
    // When
    let driver = SPUStandardUserDriver(hostBundle: bundle, delegate: nil)
    let updater = SPUUpdater(
      hostBundle: bundle, applicationBundle: Bundle.main, userDriver: driver, delegate: nil)
    // Then
    XCTAssertTrue(updater.automaticallyChecksForUpdates)
    XCTAssertTrue(updater.automaticallyDownloadsUpdates)
  }
}
