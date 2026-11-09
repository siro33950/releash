import Foundation
import ServiceManagement

@MainActor final class LoginItemRegistration {
  static let didSynchronize = Notification.Name("ReleashLoginItemDidSynchronize")
  private static let executablePathKey = "loginItemExecutablePath"
  var status: () -> SMAppService.Status = { SMAppService.mainApp.status }
  var register: () throws -> Void = { try SMAppService.mainApp.register() }
  var unregister: () throws -> Void = { try SMAppService.mainApp.unregister() }
  var defaults: UserDefaults = .standard
  var executablePath: String? = Bundle.main.executablePath

  func synchronize(autoLaunch: Bool, model: AppModel) async throws {
    try Task.checkCancellation()
    defer { NotificationCenter.default.post(name: Self.didSynchronize, object: model) }
    if !autoLaunch {
      if status() == .enabled || status() == .requiresApproval { try unregister() }
      return
    }
    guard let client = model.client else {
      throw CLIFailure(message: "サーバに接続されていません")
    }
    guard let executablePath else {
      throw CLIFailure(message: "アプリの実行ファイルのパスを取得できません")
    }
    var request = Releash_Client_V1_CheckLoginRegistrationRequest()
    request.executablePath = executablePath
    let result = try await client.checkLoginRegistration(request: request, headers: model.headers)
      .result.get()
    try Task.checkCancellation()
    guard result.status == .allowed else { return }
    if status() == .enabled && defaults.string(forKey: Self.executablePathKey) != executablePath {
      try unregister()
      try register()
      defaults.set(executablePath, forKey: Self.executablePathKey)
    } else if status() == .notRegistered || status() == .notFound {
      try register()
      defaults.set(executablePath, forKey: Self.executablePathKey)
    }
  }
}
