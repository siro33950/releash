import Connect
import SwiftUI

struct SourceControlView: View {
  var model: AppModel
  let worktree: String
  let openFile: (String) -> Void
  @State private var query = ""
  private var message: String {
    get { model.sourceControlDrafts[worktree, default: .init()].message }
    nonmutating set { model.sourceControlDrafts[worktree, default: .init()].message = newValue }
  }
  @State private var snapshot = Releash_Client_V1_SourceControlSnapshot()
  @State private var subscription: UUID?
  @State private var busy = false
  private var operationFailure: String? {
    get { model.sourceControlDrafts[worktree, default: .init()].operationFailure }
    nonmutating set {
      model.sourceControlDrafts[worktree, default: .init()].operationFailure = newValue
    }
  }
  @State private var forceConfirmation = false
  @State private var discardConfirmation = false
  @State private var discardPaths: [String] = []
  @State private var discardWarning = ""

  private var hasTracking: Bool {
    if case .tracking = snapshot.branchState { return true }
    return false
  }
  private func isRebase(_ operation: Releash_Client_V1_SourceControlOperation) -> Bool {
    if case .rebase = operation.kind { return true }
    return false
  }
  private var arguments: [String] { [worktree] + (query.isBlank ? [] : ["search=\(query)"]) }
  private var canCommit: Bool {
    snapshot.canCommit && !message.isBlank && !busy
  }
  private var operation: Releash_Client_V1_SourceControlOperation? {
    if case .operation(let value) = snapshot.branchState { return value }
    return nil
  }

  var body: some View {
    VStack(alignment: .leading, spacing: 12) {
      HStack {
        Text(snapshot.changedFileCount == 0 ? "変更なし" : "\(snapshot.changedFileCount) ファイルの変更")
        Spacer()
        IconMenu(title: "…", image: "ellipsis") {
          Button("Force Push") { forceConfirmation = true }
        }

      }
      TextField("ファイル名で絞り込み", text: $query).textFieldStyle(.roundedBorder)
      if let operation { operationBanner(operation) }
      branchRow
      Group {
        if operation?.hasMessage == false || busy {
          ReadOnlyCommitMessage(message: operation?.hasMessage == false ? "" : message)
        } else {
          TextEditor(text: Binding(get: { message }, set: { message = $0 }))
        }
      }.frame(height: 75)
        .overlay(RoundedRectangle(cornerRadius: 4).stroke(.separator))
        .accessibilityLabel("Commit メッセージ")
      HStack {
        Spacer()
        if let operation {
          if !operation.canContinue { Text("conflict をすべて解決すると続けられます").font(.caption) }
          Button(isRebase(operation) ? "Rebase を続ける" : "Merge を続ける") { continueOperation() }
            .disabled(!operation.canContinue || busy)
        } else {
          Button("Commit") { commit(.none) }.disabled(!canCommit)
          Menu("▾") {
            Button("Commit") { commit(.none) }.disabled(!canCommit)
            Button("Commit & Push\(sendCount(afterCommit: true))") { commit(.push) }.disabled(
              !canCommit)
            Button("Commit & Sync\(receiveCount)\(sendCount(afterCommit: true))") { commit(.sync) }
              .disabled(!canCommit)
            Button("Push\(sendCount())") { push() }
            Button("Pull\(receiveCount)") { pull() }
            Button("Sync\(receiveCount)\(sendCount())") { sync() }
            Button("Fetch") { fetch() }
          }.menuStyle(.button).fixedSize().disabled(busy)
        }
      }
      if let operationFailure { Text(operationFailure).foregroundStyle(.red) }
      Divider()
      ScrollView {
        VStack(alignment: .leading, spacing: 12) {
          if snapshot.changedFileCount == 0 {
            Text("Worktree に変更はありません")
            if hasTracking && snapshot.tracking.ahead > 0 {
              Button("Push（\(snapshot.tracking.ahead) commit）") { push() }.disabled(busy)
            }
          } else {
            if !snapshot.conflicts.isEmpty { conflicts }
            changes("STAGED CHANGES", files: snapshot.staged, staged: true)
            changes("CHANGES", files: snapshot.unstaged, staged: false)
          }
        }.frame(maxWidth: .infinity, alignment: .leading)
      }
    }
    .padding(12)
    .alert("Force Push", isPresented: $forceConfirmation) {
      Button("キャンセル", role: .cancel) {}
      Button("Force Push", role: .destructive) { push(force: true) }
    } message: {
      Text("\(snapshot.tracking.upstream) を --force-with-lease で上書きします。")
    }
    .alert("変更を破棄", isPresented: $discardConfirmation) {
      Button("キャンセル", role: .cancel) {}
      Button("破棄", role: .destructive) { discard() }
    } message: {
      Text(discardWarning)
    }
    .task(id: arguments) {
      if let subscription { model.unsubscribe(subscription) }
      subscription = model.subscribe("source-control", arguments) { payload in
        guard case .sourceControl(let value) = payload.value else { return }
        let previous = model.sourceControlDrafts[worktree]?.operation
        snapshot = value
        if let current = operation,
          current.message != previous?.message || current.hasMessage != previous?.hasMessage
            || current.kind != previous?.kind
        {
          message = current.message
        }
        if previous != nil && operation == nil { message = "" }
        model.sourceControlDrafts[worktree, default: .init()].operation = operation
      }
    }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
    }
  }

  private var branchRow: some View {
    VStack(alignment: .leading, spacing: 4) {
      Text(snapshot.branch).fontWeight(.medium)
      switch snapshot.branchState {
      case .tracking(let value):
        HStack {
          Text(value.upstream)
          Spacer()
          Text("↑ \(value.ahead)")
          Text("↓ \(value.behind)")
        }
      case .unpublished(let value):
        HStack {
          Text("origin にまだありません（\(value.unpushedCommits) commit）")
          Spacer()
          Button("Publish Branch") { push() }.disabled(busy)
        }
      case .operation(let value): Text(isRebase(value) ? "rebase 中" : "merge 中")
      case nil: EmptyView()
      }
    }.font(.caption)
  }

  private func operationBanner(_ value: Releash_Client_V1_SourceControlOperation) -> some View {
    VStack(alignment: .leading) {
      HStack {
        Text(isRebase(value) ? "\(value.rebase.onto) への rebase の途中です" : "merge の途中です")
        Spacer()
        Button("中止") { abortOperation() }.disabled(busy)
      }
      if isRebase(value) { Text("\(value.rebase.total) commit 中 \(value.rebase.step) つ目") }
      Text("conflict \(value.conflictCount) 件")
    }.padding(8).background(Color.orange.opacity(0.1)).accessibilityElement(children: .contain)
  }

  private var conflicts: some View {
    VStack(alignment: .leading) {
      HStack {
        Text("CONFLICTS")
        Text("\(snapshot.conflicts.count)")
      }
      .accessibilityElement(children: .contain)
      ForEach(snapshot.conflicts, id: \.path) { file in
        HStack {
          Button(file.name) { openFile(file.path) }.buttonStyle(.plain)
          Text(file.directory).foregroundStyle(.secondary)
          Text(conflictLabel(file.kind))
          Spacer()
          Button("+") { stage([file.path], staged: false) }.help("解決済みにする").disabled(busy)
        }.accessibilityElement(children: .contain)
      }
    }.accessibilityElement(children: .contain)
  }

  private func changes(
    _ title: String, files: [Releash_Client_V1_SourceControlChange], staged: Bool
  ) -> some View {
    VStack(alignment: .leading, spacing: 6) {
      HStack {
        Text(title)
        Text("\(files.count)")
        Spacer()
        if !staged { Button("破棄") { confirmDiscard(nil) }.disabled(files.isEmpty || busy) }
        Button(staged ? "すべて unstage" : "すべて stage") { stage(files.map(\.path), staged: staged) }
          .disabled(files.isEmpty || busy)
      }.font(.caption).accessibilityElement(children: .contain)
      ForEach(files, id: \.path) { file in
        SourceControlChangeRow(
          file: file, staged: staged, busy: busy,
          open: { openFile(file.path) }, stage: { stage([file.path], staged: staged) },
          discard: { confirmDiscard(file) })
      }
    }.accessibilityElement(children: .contain).accessibilityLabel(title)
  }

  private var receiveCount: String { hasTracking ? "  ↓ \(snapshot.tracking.behind)" : "" }
  private func sendCount(afterCommit: Bool = false) -> String {
    guard hasTracking else { return "" }
    return "  ↑ \(afterCommit ? snapshot.tracking.aheadAfterCommit : snapshot.tracking.ahead)"
  }
  private func conflictLabel(_ kind: Releash_Client_V1_SourceControlConflictKind) -> String {
    switch kind {
    case .bothModified: "両方で変更"
    case .bothAdded: "両方で追加"
    case .bothDeleted: "両方で削除"
    case .addedByUs: "こちらで追加"
    case .addedByThem: "相手で追加"
    case .deletedByUs: "こちらで削除"
    case .deletedByThem: "相手で削除"
    default: "conflict"
    }
  }
  private func confirmDiscard(_ file: Releash_Client_V1_SourceControlChange?) {
    if let file {
      discardPaths = [file.path]
      discardWarning =
        file.kind == .untracked
        ? "\(file.name) をゴミ箱に移します。"
        : "\(file.name) の変更（+\(file.additions) −\(file.deletions)）を破棄します。この操作は取り消せません。"
    } else {
      discardPaths = snapshot.unstaged.map(\.path)
      let summary = snapshot.discardAll
      discardWarning = "変更（+\(summary.additions) −\(summary.deletions)）を破棄します。この操作は取り消せません。"
      if summary.trashedFiles > 0 { discardWarning += " \(summary.trashedFiles) ファイルをゴミ箱に移します。" }
    }
    discardConfirmation = true
  }

  private func perform(
    _ action:
      @escaping @MainActor (any Releash_Client_V1_GitServiceClientInterface) async throws -> Void
  ) {
    guard let client = model.gitClient else { return }
    busy = true
    operationFailure = nil
    model.perform {
      defer { busy = false }
      do { try await action(client) } catch { operationFailure = model.failureMessage(error) }
    }
  }
  private func commit(_ followUp: Releash_Client_V1_GitCommitFollowUp) {
    let request = Releash_Client_V1_GitCommitRequest.with {
      $0.worktreePath = worktree
      $0.message = message
      $0.followUp = followUp
    }
    perform { client in
      do {
        _ = try await client.gitCommit(request: request, headers: model.headers).result.get()
        if message == request.message { message = "" }
      } catch {
        if let error = error as? ConnectError {
          let detail: Releash_Client_V1_CommandError? = error.unpackedDetails().first
          if case .coded(let coded) = detail?.variant,
            coded.code == "GIT_COMMIT_FOLLOW_UP_FAILED"
          {
            if message == request.message { message = "" }
          }
        }
        throw error
      }
    }
  }
  private func push(force: Bool = false) {
    let request = Releash_Client_V1_GitPushRequest.with {
      $0.worktreePath = worktree
      $0.forceWithLease = force
    }
    perform { client in
      _ = try await client.gitPush(request: request, headers: model.headers).result.get()
    }
  }
  private func pull() {
    let request = Releash_Client_V1_GitPullRequest.with { $0.worktreePath = worktree }
    perform { client in
      _ = try await client.gitPull(request: request, headers: model.headers).result.get()
    }
  }
  private func sync() {
    let request = Releash_Client_V1_GitSyncRequest.with { $0.worktreePath = worktree }
    perform { client in
      _ = try await client.gitSync(request: request, headers: model.headers).result.get()
    }
  }
  private func fetch() {
    let request = Releash_Client_V1_GitFetchRequest.with { $0.worktreePath = worktree }
    perform { client in
      _ = try await client.gitFetch(request: request, headers: model.headers).result.get()
    }
  }
  private func discard() {
    let request = Releash_Client_V1_GitDiscardRequest.with {
      $0.worktreePath = worktree
      $0.paths = discardPaths
    }
    perform { client in
      _ = try await client.gitDiscard(request: request, headers: model.headers).result.get()
    }
  }
  private func continueOperation() {
    let request = Releash_Client_V1_GitContinueOperationRequest.with {
      $0.worktreePath = worktree
      $0.message = message
    }
    perform { client in
      _ = try await client.gitContinueOperation(request: request, headers: model.headers).result
        .get()
    }
  }
  private func abortOperation() {
    let request = Releash_Client_V1_GitAbortOperationRequest.with { $0.worktreePath = worktree }
    perform { client in
      _ = try await client.gitAbortOperation(request: request, headers: model.headers).result.get()
    }
  }
  private func stage(_ paths: [String], staged: Bool) {
    if staged {
      let request = Releash_Client_V1_GitUnstageRequest.with {
        $0.repoPath = worktree
        $0.paths = .with { $0.items = paths }
      }
      perform { client in
        _ = try await client.gitUnstage(request: request, headers: model.headers).result.get()
      }
    } else {
      let request = Releash_Client_V1_GitStageRequest.with {
        $0.repoPath = worktree
        $0.paths = .with { $0.items = paths }
      }
      perform { client in
        _ = try await client.gitStage(request: request, headers: model.headers).result.get()
      }
    }
  }
}

private struct SourceControlChangeRow: View {
  let file: Releash_Client_V1_SourceControlChange
  let staged: Bool
  let busy: Bool
  let open: () -> Void
  let stage: () -> Void
  let discard: () -> Void
  @State private var hovering = false
  @FocusState private var actionsFocused: Bool
  private var status: String {
    switch file.kind {
    case .modified: "M"
    case .added: "A"
    case .deleted: "D"
    case .untracked: "U"
    default: ""
    }
  }
  var body: some View {
    HStack {
      HStack {
        Text(status)
        Button(file.name, action: open).buttonStyle(.plain)
        Text(file.directory).foregroundStyle(.secondary)
        Spacer()
        Text("+\(file.additions)").foregroundStyle(.green)
        Text("−\(file.deletions)").foregroundStyle(.red)
      }
      .contentShape(Rectangle())
      .onTapGesture(perform: open)
      .accessibilityElement(children: .contain)
      .accessibilityAction(.default, open)
      HStack {
        if !staged { Button("破棄", action: discard).focused($actionsFocused) }
        Button(staged ? "unstage" : "stage", action: stage).focused($actionsFocused)
      }.buttonStyle(.plain).foregroundStyle(
        hovering || actionsFocused ? Color.primary : Color.clear
      ).disabled(busy)
    }
    .onHover { hovering = $0 }
    .accessibilityElement(children: .contain)
  }
}

private struct ReadOnlyCommitMessage: NSViewRepresentable {
  let message: String
  func makeNSView(context: Context) -> NSScrollView {
    let scrollView = NSTextView.scrollableTextView()
    if let textView = scrollView.documentView as? NSTextView {
      textView.isEditable = false
      textView.string = message
    }
    return scrollView
  }

  func updateNSView(_ view: NSScrollView, context: Context) {
    (view.documentView as? NSTextView)?.string = message
  }
}
