import Bonsplit
import SwiftUI

struct FileWorkspaceView: View {
  var model: AppModel
  let worktree: String
  @State private var query = ""
  @State private var changed = false
  @State private var snapshot = Releash_Client_V1_FilteredFilesSnapshot()
  @State private var subscription: UUID?
  private var filtering: Bool { !query.isEmpty || changed }
  private var arguments: [String] {
    [worktree] + (query.isEmpty ? [] : ["name=\(query)"]) + (changed ? ["changed=true"] : [])
  }

  var body: some View {
    WorkspaceColumnSplit(
      minimumLeadingWidth: 180, minimumTrailingWidth: 240, initialLeadingWidth: 260,
      dividerThickness: 1
    ) {
      VStack(spacing: 0) {
        HStack(spacing: 4) {
          TextField("検索", text: $query).textFieldStyle(.roundedBorder)
            .accessibilityIdentifier("file-search")
          Toggle(isOn: $changed) { CardLabel(title: "差分のみ", image: "diff") }
            .toggleStyle(IconToggleStyle()).labelStyle(.iconOnly).iconButtonFrame().help("差分のみ")
          Button(action: openInEditor) {
            CardLabel(title: "IDE で開く", image: "square-arrow-out-up-right")
          }.labelStyle(.iconOnly).buttonStyle(IconButtonStyle()).iconButtonFrame().help("IDE で開く")
        }.padding(8).frame(height: 44).background(Color(nsColor: .textBackgroundColor))
        if filtering && snapshot.truncated {
          Text("一致した結果の一部を表示しています。入力を増やして絞り込んでください。")
            .font(.caption).padding(8)
        }
        Divider()
        FileTreeView(model: model, worktree: worktree, filtered: filtering ? snapshot : nil)
      }.frame(minWidth: 180, idealWidth: 260)
    } trailing: {
      BonsplitView(controller: model.filePanes.controller) { tab, _ in
        if let file = model.filePanes.tab(tab.id), let path = file.filePath {
          FileTabPane(model: model, tabID: tab.id, path: path, worktree: worktree)
            .id("\(worktree):\(tab.id):\(path)")
        }
      } emptyPane: { _ in
        Color(nsColor: .textBackgroundColor)
      }
      .background(WorkspaceFocusObserver { model.filesFocused = true })
      .frame(minWidth: 240)
    }
    .onAppear {
      model.filePanes.onFocus = { [weak model] in model?.filesFocused = true }
    }
    .task(id: arguments) {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
      snapshot = Releash_Client_V1_FilteredFilesSnapshot()
      guard filtering else { return }
      do {
        if !query.isEmpty { try await Task.sleep(for: .milliseconds(250)) }
        try Task.checkCancellation()
        subscription = model.subscribe("filtered-files", arguments) { payload in
          if case .filteredFiles(let value) = payload.value { snapshot = value }
        }
      } catch is CancellationError {
      } catch { model.failure = error.localizedDescription }
    }
    .onChange(of: model.filePanes.layoutInput) { _, _ in model.saveFilePaneLayout() }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
      model.filesFocused = false
    }
  }

  private func openInEditor() {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_OpenInEditorRequest()
    request.worktreePath = worktree
    model.perform {
      _ = try await client.openInEditor(request: request, headers: model.headers).result.get()
    }
  }
}
