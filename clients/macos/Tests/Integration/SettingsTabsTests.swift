import XCTest

@testable import Releash

final class SettingsTabsTests: XCTestCase {
  @MainActor func test_Settings_各タブを選択して対応する設定を表示する() async throws {
    // Given
    let model = AppModel()
    let host = await ViewHarness(SettingsView(model: model))
    defer { host.close() }
    for (title, content) in [
      ("Repositories", "リポジトリを選択してください"),
      ("Providers", "settings-provider-refresh"),
      ("Workflows", "settings-workflows-mode"),
      ("System", "settings-auto-launch"),
      ("General", "settings-terminal-command"),
    ] {
      // When
      try await host.press(title)
      // Then
      try await eventuallyView(host) { host.element(content) != nil }
    }
  }
}
