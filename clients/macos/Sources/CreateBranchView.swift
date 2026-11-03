import SwiftUI

struct CreateBranchView: View {
  let model: AppModel
  @State private var name = ""
  @State private var failure: String?
  @State private var submitting = false
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      Text("ブランチを作成").font(.headline)
      TextField("ブランチ名", text: $name)
        .accessibilityIdentifier("create-branch-name")
      if let failure { Text(failure).foregroundStyle(.red) }
      HStack {
        Spacer()
        Button("キャンセル") { dismiss() }.keyboardShortcut(.cancelAction)
          .disabled(submitting)
        Button("作成") {
          submitting = true
          failure = nil
          Task {
            defer { submitting = false }
            do {
              try await model.createBranch(name)
              dismiss()
            } catch {
              failure = model.failureMessage(error)
            }
          }
        }
        .keyboardShortcut(.defaultAction)
        .disabled(submitting || name.isEmpty || model.selectedWorktree == nil || !model.connected)
      }
    }
    .padding(UIStyle.padding)
    .frame(width: UIStyle.createDialogWidth)
  }
}
