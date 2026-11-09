import ServiceManagement
import XCTest

@testable import Releash

final class LoginItemRegistrationRetryTests: XCTestCase {
  @MainActor func test_ログイン項目_再登録に失敗したパスは保存せず次回の同期で登録する() async throws {
    // Given
    let http = TestHTTPClient()
    http.state.withLock { $0.loginRegistration.status = .allowed }
    let model = try await testModel(http)
    let item = LoginItemRegistration()
    let suite = "LoginItemRegistrationRetryTests.\(UUID().uuidString)"
    item.defaults = try XCTUnwrap(UserDefaults(suiteName: suite))
    defer { item.defaults.removePersistentDomain(forName: suite) }
    let oldPath = "/Applications/Releash Preview.app/Contents/MacOS/Releash Preview"
    item.defaults.set(oldPath, forKey: "loginItemExecutablePath")
    item.executablePath = "/Applications/Releash.app/Contents/MacOS/Releash"
    var status: SMAppService.Status = .enabled
    var operations: [String] = []
    var failRegistration = true
    item.status = { status }
    item.unregister = {
      operations.append("unregister")
      status = .notRegistered
    }
    item.register = {
      operations.append("register")
      if failRegistration { throw CLIFailure(message: "register failed") }
      status = .enabled
    }
    // When
    do {
      try await item.synchronize(autoLaunch: true, model: model)
      XCTFail("再登録の失敗が返りませんでした")
    } catch {
      // Then
      XCTAssertEqual(model.failureMessage(error), "register failed")
    }
    XCTAssertEqual(operations, ["unregister", "register"])
    XCTAssertEqual(item.defaults.string(forKey: "loginItemExecutablePath"), oldPath)
    // When
    failRegistration = false
    try await item.synchronize(autoLaunch: true, model: model)
    try await item.synchronize(autoLaunch: true, model: model)
    // Then
    XCTAssertEqual(operations, ["unregister", "register", "register"])
    XCTAssertEqual(item.defaults.string(forKey: "loginItemExecutablePath"), item.executablePath)
    await model.stop()
  }
}
