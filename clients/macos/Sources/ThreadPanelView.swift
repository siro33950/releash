import Observation
import SwiftUI
import Textual

struct ThreadPanelView: View {
  var model: AppModel
  let worktree: String
  @AppStorage("diffBase") private var diffBase = "head"
  @State private var subscription: UUID?

  var state: ThreadPanelState { model.threadPanelState(worktree) }
  var sessionNodes: [Releash_Client_V1_WorkspaceNode] {
    Self.sessionNodes(
      model.repositories.flatMap(\.worktrees.items).first { $0.path == worktree }?.snapshot.nodes
        .items ?? [])
  }
  var openSessions: [Releash_Client_V1_WorkspaceNode] {
    sessionNodes.filter { $0.processPresence.value == .live }
  }
  /// 購読の引数。worktree、差分の比較の基準、解決済みでも差分を求める Thread を並べる。
  var subscriptionArguments: [String] {
    [worktree, diffBase] + state.expandedResolvedIDs.map { "thread=\($0)" }
  }

  var body: some View {
    VStack(spacing: 0) {
      header
      Divider()
      ScrollView {
        LazyVStack(alignment: .leading, spacing: UIStyle.spacing) {
          ForEach(state.openItems, id: \.thread.id) { card($0) }
          if !state.resolvedItems.isEmpty {
            Text("解決済み").font(.subheadline).foregroundStyle(.secondary)
            ForEach(state.resolvedItems, id: \.thread.id) { item in
              if state.expanded.contains(item.thread.id) {
                card(item)
              } else {
                ResolvedThreadRow(thread: item.thread) { state.expanded.insert(item.thread.id) }
              }
            }
          }
          generalComposer
        }
        .padding(UIStyle.spacing)
      }
    }
    .environment(
      \.openURL,
      OpenURLAction { url in
        openCommentLink(url) ? .handled : .discarded
      }
    )
    .onAppear { subscribe() }
    .onChange(of: subscriptionArguments) { _, _ in subscribe() }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
    }
  }

  private func subscribe() {
    if let subscription { model.unsubscribe(subscription) }
    let state = state
    subscription = model.subscribe("review-threads", subscriptionArguments) { payload in
      if case .threadExcerpts(let value) = payload.value { state.receive(value.items) }
    }
  }

  private var header: some View {
    HStack(spacing: UIStyle.spacing) {
      (Text("未解決 \(state.openItems.count)").fontWeight(.semibold)
        + Text(" · 解決済み \(state.resolvedItems.count)").foregroundStyle(.secondary))
        .lineLimit(1)
        .accessibilityIdentifier("thread-counts")
      Spacer(minLength: 0)
      Text("\(state.selectedIDs.count) 件を選択").foregroundStyle(.secondary).lineLimit(1)
        .layoutPriority(1)
        .accessibilityIdentifier("thread-selected-count")
      Menu {
        ForEach(openSessions, id: \.sessionID) { node in
          Button(node.title) { send(to: node.sessionID) }
            .accessibilityIdentifier("thread-session:\(node.sessionID)")
        }
      } label: {
        CardLabel(title: "Session に送る", image: "send")
      }
      .menuStyle(.button).fixedSize()
      .disabled(state.selectedIDs.isEmpty || openSessions.isEmpty)
      .accessibilityIdentifier("thread-send")
    }
    .padding(UIStyle.spacing)
  }

  private func card(_ item: Releash_Client_V1_ThreadExcerpt) -> some View {
    let id = item.thread.id
    let state = state
    return ThreadCard(
      item: item,
      checked: Binding(
        get: { state.checked.contains(id) },
        set: { if $0 { state.checked.insert(id) } else { state.checked.remove(id) } }),
      draft: Binding(get: { state.drafts[id, default: ""] }, set: { state.drafts[id] = $0 }),
      sending: state.sending.contains(id),
      reply: { sendReply(id) },
      resolve: { resolve(id) },
      collapse: { state.expanded.remove(id) })
  }

  private var generalComposer: some View {
    @Bindable var state = state
    return VStack(alignment: .leading, spacing: 0) {
      Text("General に追加").fontWeight(.semibold).padding(UIStyle.spacing)
      Divider()
      HStack(alignment: .bottom, spacing: UIStyle.spacing) {
        CommentField(
          placeholder: "コメントを書く…（Markdown）", text: $state.generalDraft,
          identifier: "thread-general-content", submit: addGeneral)
        Button("追加", action: addGeneral)
          .disabled(state.generalDraft.isBlank || state.creatingGeneral)
          .accessibilityIdentifier("thread-general-create")
      }
      .padding(UIStyle.spacing)
    }
    .threadCard()
  }

  static func sessionNodes(_ items: [Releash_Client_V1_WorkspaceTreeItem])
    -> [Releash_Client_V1_WorkspaceNode]
  {
    items.flatMap { item in
      let nodes: [Releash_Client_V1_WorkspaceNode]
      if case .node(let node) = item.variant, node.hasSessionID {
        nodes = [node]
      } else {
        nodes = []
      }
      return nodes + sessionNodes(item.children) + sessionNodes(item.pastAttempts)
    }
  }

  /// 書きかけの返信を送る。送れたら、その間に書き足されていない限り入力欄を空にする。
  func sendReply(_ id: String) {
    let state = state
    let content = state.drafts[id, default: ""]
    guard !content.isBlank, state.sending.insert(id).inserted else { return }
    reply(to: id, content: content) { accepted in
      state.sending.remove(id)
      if accepted, state.drafts[id] == content { state.drafts[id] = nil }
    }
  }

  /// 書きかけの本文で General の Thread を作る。作れたら、その間に書き足されていない限り入力欄を空にする。
  func addGeneral() {
    let state = state
    let content = state.generalDraft
    guard !content.isBlank, !state.creatingGeneral else { return }
    state.creatingGeneral = true
    createGeneral(content: content) { accepted in
      state.creatingGeneral = false
      if accepted, state.generalDraft == content { state.generalDraft = "" }
    }
  }

  func reply(to id: String, content: String, completed: @escaping (Bool) -> Void) {
    guard let client = model.client else { return completed(false) }
    var request = Releash_Client_V1_AppendReviewCommentRequest()
    request.worktreeName = worktree
    request.threadID = id
    request.content = content
    model.perform {
      do {
        _ = try await client.appendReviewComment(request: request, headers: model.headers).result
          .get()
        completed(true)
      } catch {
        completed(false)
        throw error
      }
    }
  }

  func resolve(_ id: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_ResolveReviewThreadRequest()
    request.worktreeName = worktree
    request.threadID = id
    request.outcome = "resolved"
    request.summary = "Threadの面で解決"
    model.perform {
      _ = try await client.resolveReviewThread(request: request, headers: model.headers).result
        .get()
    }
  }

  func createGeneral(content: String, completed: @escaping (Bool) -> Void) {
    guard let client = model.client else { return completed(false) }
    var request = Releash_Client_V1_CreateReviewThreadRequest()
    request.worktreeName = worktree
    request.content = content
    model.perform {
      do {
        _ = try await client.createReviewThread(request: request, headers: model.headers).result
          .get()
        completed(true)
      } catch {
        completed(false)
        throw error
      }
    }
  }

  func send(to sessionID: String) {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_SendThreadsToSessionRequest()
    request.agentSessionID = sessionID
    request.threadIds.items = state.selectedIDs
    model.perform {
      _ = try await client.sendThreadsToSession(request: request, headers: model.headers).result
        .get()
    }
  }
}

/// Thread 面の状態。面を離れても保つので、worktree ごとに `AppModel` が持つ。
@Observable @MainActor final class ThreadPanelState {
  var items: [Releash_Client_V1_ThreadExcerpt] = []
  var checked: Set<String> = []
  /// カードの形に開いている、解決済みの Thread。
  var expanded: Set<String> = []
  var drafts: [String: String] = [:]
  var sending: Set<String> = []
  var generalDraft = ""
  var creatingGeneral = false
  var openItems: [Releash_Client_V1_ThreadExcerpt] { items.filter { !$0.thread.isResolved } }
  var resolvedItems: [Releash_Client_V1_ThreadExcerpt] { items.filter(\.thread.isResolved) }
  var selectedIDs: [String] { items.map(\.thread.id).filter(checked.contains) }
  /// カードの形に開いている解決済みの Thread の ID。購読の引数にするので順を固定する。
  var expandedResolvedIDs: [String] {
    resolvedItems.map(\.thread.id).filter(expanded.contains).sorted()
  }

  func receive(_ items: [Releash_Client_V1_ThreadExcerpt]) {
    self.items = items
    let ids = Set(items.map(\.thread.id))
    checked.formIntersection(ids)
    expanded.formIntersection(ids)
    drafts = drafts.filter { ids.contains($0.key) }
  }
}

extension String {
  var isBlank: Bool { trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
}

extension Releash_Client_V1_ReviewThreadDto {
  var isResolved: Bool { state.value == .resolved }
  var pathLabel: String { target.hasFilePath ? target.filePath : "General" }
  /// 対象の行。`L46`、範囲なら `L12–18`。行を持たない対象では `nil`。
  var lineLabel: String? {
    guard target.hasLineNumber else { return nil }
    let end = target.hasEndLine && target.endLine != target.lineNumber ? "–\(target.endLine)" : ""
    return "L\(target.lineNumber)\(end)"
  }
  /// 1 行にたたんだときの場所。ファイルはパスの最後の名前だけにする。
  var compactLocationLabel: String {
    let name = target.hasFilePath ? (target.filePath as NSString).lastPathComponent : "General"
    return [name, lineLabel].compactMap { $0 }.joined(separator: " ")
  }
}

/// 今からさかのぼった時間を「25 分前」の形にする。ファイルタブの Thread と同じ区切りで表す。
func relativeTimeLabel(_ timestamp: Double, now: Date) -> String {
  let elapsed = now.timeIntervalSince1970 - timestamp
  guard elapsed.isFinite else { return "" }
  let seconds = Int(max(0, min(elapsed, Double(Int32.max))))
  for (unit, size) in [("年", 31_536_000), ("日", 86_400), ("時間", 3_600), ("分", 60)]
  where seconds >= size {
    return "\(seconds / size) \(unit)前"
  }
  return "たった今"
}

/// 相対時刻。表示したままでも 1 分ごとに進む。
private struct RelativeTimeText: View {
  let timestamp: Double
  var prefix = ""
  var body: some View {
    TimelineView(.everyMinute) { _ in
      Text(prefix + relativeTimeLabel(timestamp, now: Date()))
    }
  }
}

extension View {
  /// Thread 面の 1 枚のカードの枠。
  fileprivate func threadCard() -> some View {
    frame(maxWidth: .infinity, alignment: .leading)
      .clipShape(RoundedRectangle(cornerRadius: UIStyle.cardCornerRadius))
      .overlay(RoundedRectangle(cornerRadius: UIStyle.cardCornerRadius).strokeBorder(.separator))
  }
}

/// Markdown の Comment を書く欄。Return で送り、⌥Return で改行する。
///
/// 入力に合わせて `UIStyle.commentInputLines` 行まで伸び、それを超えると中でスクロールする。
struct CommentField: View {
  let placeholder: String
  @Binding var text: String
  let identifier: String
  let submit: () -> Void

  var body: some View {
    TextField(placeholder, text: $text, axis: .vertical)
      .lineLimit(1...UIStyle.commentInputLines).textFieldStyle(.roundedBorder)
      .onSubmit(submit)
      .accessibilityIdentifier(identifier)
  }
}

private struct ThreadCard: View {
  let item: Releash_Client_V1_ThreadExcerpt
  @Binding var checked: Bool
  @Binding var draft: String
  let sending: Bool
  let reply: () -> Void
  let resolve: () -> Void
  let collapse: () -> Void
  private var thread: Releash_Client_V1_ReviewThreadDto { item.thread }

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      HStack(spacing: UIStyle.spacing) {
        Toggle("送信するThread", isOn: $checked).toggleStyle(.checkbox).labelsHidden()
          .accessibilityIdentifier("thread-check:\(thread.id)")
        Text(thread.pathLabel).font(.system(.callout, design: .monospaced)).fontWeight(.medium)
          .lineLimit(1).truncationMode(.head).help(thread.pathLabel).textSelection(.enabled)
          .accessibilityIdentifier("thread-path:\(thread.id)")
        if let line = thread.lineLabel {
          Text(line).font(.system(.caption, design: .monospaced)).foregroundStyle(.secondary)
            .fixedSize()
            .accessibilityIdentifier("thread-line:\(thread.id)")
        }
        Spacer(minLength: 0)
        if thread.isResolved {
          Button("隠す", action: collapse).controlSize(.small)
            .accessibilityIdentifier("thread-toggle:\(thread.id)")
        }
      }
      .padding(UIStyle.spacing)
      if item.hasReadError {
        Divider()
        Text("差分を読めません: \(item.readError)").font(.caption).foregroundStyle(.secondary)
          .textSelection(.enabled).padding(UIStyle.spacing)
          .accessibilityIdentifier("thread-diff-error:\(thread.id)")
      } else if !item.lines.isEmpty {
        Divider()
        ThreadDiffView(threadID: thread.id, lines: item.lines)
      }
      ForEach(thread.comments.items, id: \.id) { comment in
        Divider()
        ThreadCommentView(
          comment: comment,
          bodyID: comment.id == thread.comments.items.first?.id
            ? "thread-body:\(thread.id)" : "thread-reply-body:\(comment.id)")
      }
      Divider()
      if thread.isResolved {
        ResolutionLabel(thread: thread).padding(UIStyle.spacing)
      } else {
        HStack(alignment: .bottom, spacing: UIStyle.spacing) {
          CommentField(
            placeholder: "返信を書く…", text: $draft,
            identifier: "thread-reply-content:\(thread.id)", submit: reply)
          if !draft.isBlank {
            Button("返信", action: reply).disabled(sending)
              .accessibilityIdentifier("thread-reply-send:\(thread.id)")
          }
          Button("解決にする", action: resolve).disabled(!thread.canResolve || !draft.isBlank)
            .help(draft.isBlank ? "" : "書きかけの返信を送るか消すと、解決にできます")
            .accessibilityIdentifier("thread-resolve:\(thread.id)")
        }
        .padding(UIStyle.spacing)
      }
    }
    .threadCard()
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("thread:\(thread.id)")
  }
}

/// 解決済みの Thread を 1 行にたたんだ表示。
private struct ResolvedThreadRow: View {
  let thread: Releash_Client_V1_ReviewThreadDto
  let expand: () -> Void

  var body: some View {
    HStack(spacing: UIStyle.spacing) {
      ResolutionLabel(thread: thread, location: thread.compactLocationLabel)
        .help(thread.pathLabel)
      Spacer(minLength: 0)
      Button("表示", action: expand).controlSize(.small)
        .accessibilityIdentifier("thread-toggle:\(thread.id)")
    }
    .padding(UIStyle.spacing)
    .threadCard()
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("thread:\(thread.id)")
  }
}

/// 解決の印と、解決にした人と、解決からの時間。`location` を渡すと、印の後ろに場所を出す。
private struct ResolutionLabel: View {
  let thread: Releash_Client_V1_ReviewThreadDto
  var location: String?

  var body: some View {
    HStack(spacing: UIStyle.spacing) {
      CardIcon(image: Image("circle-check")).foregroundStyle(.secondary)
        .accessibilityLabel("解決済み")
        .accessibilityIdentifier("thread-resolved:\(thread.id)")
      if let location {
        Text(location).font(.system(.callout, design: .monospaced)).lineLimit(1)
          .truncationMode(.head)
      }
      RelativeTimeText(
        timestamp: thread.resolve.resolvedAt,
        prefix: "\(thread.resolve.actor.displayName) が解決にしました · "
      )
      .font(.caption).foregroundStyle(.secondary).lineLimit(1)
      .accessibilityIdentifier("thread-resolution:\(thread.id)")
    }
  }
}

/// Thread の対象の行と、その前の数行の差分。
private struct ThreadDiffView: View {
  let threadID: String
  let lines: [Releash_Client_V1_ThreadDiffLine]

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(lines.enumerated()), id: \.offset) { index, line in
        HStack(alignment: .firstTextBaseline, spacing: 0) {
          Text(line.hasOldLine ? String(line.oldLine) : "")
            .frame(width: UIStyle.diffLineNumberWidth, alignment: .trailing)
          Text(line.hasNewLine ? String(line.newLine) : "")
            .frame(width: UIStyle.diffLineNumberWidth, alignment: .trailing)
          Text(line.sign).frame(width: UIStyle.diffSignWidth)
          Text(line.text).foregroundStyle(.primary).textSelection(.enabled)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .foregroundStyle(.secondary)
        .padding(.vertical, UIStyle.diffLineSpacing)
        .background(line.tint)
        .overlay(alignment: .leading) {
          if line.targeted {
            Rectangle().fill(Color.accentColor).frame(width: UIStyle.diffTargetMarkerWidth)
          }
        }
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(verbatim: line.text))
        .accessibilityAddTraits(.isStaticText)
        .accessibilityValue(line.accessibilityDescription)
        .accessibilityIdentifier("thread-diff-line:\(threadID):\(index)")
      }
    }
    .font(.system(.caption, design: .monospaced))
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("thread-diff:\(threadID)")
  }
}

extension Releash_Client_V1_ThreadDiffLine {
  var sign: String {
    switch (hasOldLine, hasNewLine) {
    case (false, true): "+"
    case (true, false): "-"
    default: " "
    }
  }
  /// 読み上げ用の、行の種類と指摘の有無。
  var accessibilityDescription: String {
    let kind: String? =
      switch sign {
      case "+": "追加"
      case "-": "削除"
      default: nil
      }
    return [kind, targeted ? "指摘された行" : nil].compactMap { $0 }.joined(separator: "、")
  }
  fileprivate var tint: Color {
    switch (hasOldLine, hasNewLine) {
    case (false, true): UIStyle.diffAddition.opacity(UIStyle.diffTintOpacity)
    case (true, false): UIStyle.diffDeletion.opacity(UIStyle.diffTintOpacity)
    default: targeted ? Color.accentColor.opacity(UIStyle.diffTintOpacity) : .clear
    }
  }
}

private struct ThreadCommentView: View {
  let comment: Releash_Client_V1_ReviewCommentDto
  let bodyID: String
  private var agent: Bool { comment.author.kind.value == .agent }

  var body: some View {
    HStack(alignment: .top, spacing: UIStyle.spacing) {
      Group {
        if agent {
          CardIcon(image: Image("bot")).accessibilityLabel("agent")
        } else {
          Text(comment.author.displayName.prefix(1).uppercased()).font(.caption)
            .fontWeight(.semibold).accessibilityHidden(true)
        }
      }
      .frame(width: UIStyle.avatarSize, height: UIStyle.avatarSize)
      .background(Color(nsColor: .quaternarySystemFill), in: Circle())
      VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
        HStack(spacing: UIStyle.spacing) {
          Text(comment.author.displayName).fontWeight(.semibold).lineLimit(1)
          if agent, comment.author.hasModel {
            Text(comment.author.model).font(.caption2).foregroundStyle(.secondary)
              .padding(.horizontal, UIStyle.rowSpacing)
              .overlay(
                RoundedRectangle(cornerRadius: UIStyle.badgeCornerRadius).strokeBorder(.separator)
              )
              .accessibilityIdentifier("thread-comment-model:\(comment.id)")
          }
          RelativeTimeText(timestamp: comment.createdAt).font(.caption)
            .foregroundStyle(.secondary)
        }
        StructuredText(markdown: comment.content).accessibilityIdentifier(bodyID)
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .padding(UIStyle.spacing)
    .accessibilityElement(children: .contain)
    .accessibilityIdentifier("thread-comment:\(comment.id)")
  }
}

@MainActor @discardableResult
func openCommentLink(_ url: URL, open: (URL) -> Bool = { NSWorkspace.shared.open($0) }) -> Bool {
  guard ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "") else { return false }
  return open(url)
}
