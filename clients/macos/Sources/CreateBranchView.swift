import SwiftUI

struct CreateBranchView: View {
  let model: AppModel
  @State private var name = ""
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      Text("ブランチを作成").font(.headline)
      TextField("ブランチ名", text: $name)
        .accessibilityIdentifier("create-branch-name")
      HStack {
        Spacer()
        Button("キャンセル") { dismiss() }.keyboardShortcut(.cancelAction)
        Button("作成") {
          model.createBranch(name)
          dismiss()
        }
        .keyboardShortcut(.defaultAction)
        .disabled(name.isEmpty || model.selectedWorktree == nil || !model.connected)
      }
    }
    .padding(UIStyle.padding)
    .frame(width: UIStyle.createDialogWidth)
  }
}
