import Connect

@MainActor protocol SettingsSaving: AnyObject {
  associatedtype Field: Hashable
  var failure: String? { get set }
  var saving: Set<Field> { get set }
}

extension SettingsSaving {
  func saveSetting(
    _ field: Field, _ model: AppModel, cleanup: () -> Void = {},
    operation: (any Releash_Client_V1_ClientServiceClientInterface, Headers) async throws -> Void
  ) async {
    guard saving.insert(field).inserted else { return }
    failure = nil
    defer {
      saving.remove(field)
      cleanup()
    }
    do {
      guard let client = model.client, model.connected else {
        throw CLIFailure(message: "サーバに接続されていません")
      }
      try await operation(client, model.headers)
    } catch {
      failure = model.failureMessage(error)
    }
  }
}
