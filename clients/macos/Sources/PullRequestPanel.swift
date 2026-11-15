import SwiftUI
import Textual

struct PullRequestPanel: View {
  var model: AppModel
  let worktree: String
  @Environment(\.openURL) private var openURL
  @State private var snapshot = Releash_Client_V1_PullRequestSnapshot()
  @State private var bases = Releash_Client_V1_PullRequestBasesSnapshot()
  @State private var subscriptions: [UUID] = []
  @State private var title = ""
  @State private var description = ""
  @State private var base = ""
  @State private var draft = false
  @State private var creating = false
  @State private var comment = ""
  @State private var error: String?
  @State private var busy = false
  @State private var passedExpanded = false
  @State private var skippedExpanded = false

  var body: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      if let error { Text(error).foregroundStyle(.red) }
      if snapshot.hasReadError { Text(snapshot.readError).foregroundStyle(.red) }
      switch snapshot.branchPullRequest {
      case .absent: creation
      case .present(let pr):
        if creating {
          creation
        } else {
          ScrollView {
            VStack(alignment: .leading, spacing: UIStyle.spacing) {
              HStack {
                let appearance = UIStyle.prAppearance(appearanceState(pr.state))
                CardIcon(image: Image(appearance.icon))
                  .foregroundStyle(appearance.color)
                Text(pr.title).font(.headline)
                Text("#" + String(pr.number)).foregroundStyle(.secondary)
                Spacer()
                Button {
                  if let url = URL(string: pr.url) { openURL(url) }
                } label: {
                  Image("square-arrow-out-up-right")
                }
                .accessibilityLabel("ブラウザで開く")
              }
              StructuredText(markdown: pr.body)
                .environment(
                  \.openURL, OpenURLAction { openCommentLink($0) ? .handled : .discarded })
              if pr.canCreateNew {
                Button("新しい PR を作る") { Task { @MainActor in creating = true } }
              }
              checks(pr.checks)
              HStack {
                Text("COMMENTS")
                Text("\(pr.comments.count)")
              }
              ForEach(Array(pr.comments.enumerated()), id: \.offset) { _, item in
                VStack(alignment: .leading) {
                  HStack {
                    if item.bot {
                      CardIcon(image: Image("bot")).accessibilityHidden(true)
                    } else {
                      Text(String(item.author.prefix(1)))
                        .frame(width: 24, height: 24).background(.quaternary, in: Circle())
                    }
                    Text(item.author)
                    if item.bot { Text("bot").font(.caption) }
                    TimelineView(.everyMinute) { _ in
                      Text(relativeTimeLabel(Double(item.writtenAt), now: Date()))
                        .foregroundStyle(.secondary)
                    }
                  }
                  StructuredText(markdown: item.body)
                    .environment(
                      \.openURL, OpenURLAction { openCommentLink($0) ? .handled : .discarded })
                }
              }
            }
          }
          Divider()
          CommentField(placeholder: "Comment", text: $comment, identifier: "pr-comment") {
            if !comment.isEmpty && !busy { post(pr.number) }
          }.accessibilityLabel("Comment")
          Button("Comment") { post(pr.number) }.disabled(comment.isEmpty || busy)
        }
      case nil: ProgressView()
      }
    }
    .padding(UIStyle.spacing)
    .textual.unorderedListMarker(PullRequestBullet())
    .textual.inlineStyle(.default.code(.font(.custom("Menlo", size: 12))))
    .onAppear {
      subscriptions = [
        model.subscribe("pull-request", [worktree]) { payload in
          if case .pullRequest(let value) = payload.value { snapshot = value }
        },
        model.subscribe("pull-request-bases", [worktree]) { payload in
          if case .pullRequestBases(let value) = payload.value {
            bases = value
            if base.isEmpty { base = value.hasDefaultBranch ? value.defaultBranch : "" }
          }
        },
      ]
    }
    .onDisappear {
      for subscription in subscriptions { model.unsubscribe(subscription) }
      subscriptions = []
    }
  }

  private var creation: some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      Text("このブランチの PR はありません")
      TextField("Title", text: $title).accessibilityLabel("Title")
      Text("Description（任意）")
      TextField("Description（任意）", text: $description, axis: .vertical).lineLimit(3...8)
        .accessibilityLabel("Description（任意）")
      Picker("base", selection: $base) {
        ForEach(bases.branches, id: \.self) { Text($0).tag($0) }
      }
      .labelsHidden()
      Toggle("draft で作る", isOn: $draft)
      Button("PR を作成", action: create).disabled(title.isEmpty || busy)
      Spacer()
    }
  }

  private func checks(_ checks: Releash_Client_V1_PullRequestChecks) -> some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      HStack {
        Text("CHECKS")
        if checks.failed > 0 { Text("失敗 \(checks.failed)") }
        if checks.cancelled > 0 { Text("取り消し \(checks.cancelled)") }
        if checks.pending > 0 { Text("実行中 \(checks.pending)") }
        if checks.skipped > 0 { Text("skip \(checks.skipped)") }
        if checks.passed > 0 { Text("通過 \(checks.passed)") }
      }
      ForEach(Array(checks.listed.enumerated()), id: \.offset) { _, check in
        HStack {
          Image(
            check.bucket == .fail
              ? "circle-alert" : check.bucket == .cancel ? "triangle-alert" : "refresh-cw"
          )
          .resizable().renderingMode(.template).scaledToFit().frame(width: 16, height: 16)
          Text(check.name)
          if check.hasFailureSeconds { Text("\(check.failureSeconds / 60) 分で失敗") }
          if check.hasStartedAt {
            TimelineView(.everyMinute) { _ in
              Text(
                "実行中 \(Int(max(0, Date().timeIntervalSince1970 - Double(check.startedAt))) / 60) 分"
              )
            }
          }
          if check.hasLogJobID {
            Button("ログを開く") {
              model.panes.openCheckLog(worktree: worktree, job: check.logJobID, title: check.name)
            }
          }
        }
      }
      if checks.passed > 0 {
        Button("通過した \(checks.passed) 件") { Task { @MainActor in passedExpanded.toggle() } }
        if passedExpanded {
          ForEach(Array(checks.passedNames.enumerated()), id: \.offset) { _, name in Text(name) }
        }
      }
      if checks.skipped > 0 {
        Button("skip した \(checks.skipped) 件") { Task { @MainActor in skippedExpanded.toggle() } }
        if skippedExpanded {
          ForEach(Array(checks.skippedNames.enumerated()), id: \.offset) { _, name in Text(name) }
        }
      }
    }
  }

  private func create() {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_CreatePullRequestRequest()
    request.worktreePath = worktree
    request.title = title
    request.body = description
    request.base = base
    request.draft = draft
    busy = true
    error = nil
    Task { @MainActor in
      defer { busy = false }
      do {
        _ = try await client.createPullRequest(request: request, headers: model.headers).result
          .get()
        creating = false
      } catch { self.error = model.failureMessage(error) }
    }
  }

  private func post(_ number: UInt64) {
    guard let client = model.client else { return }
    let sent = comment
    var request = Releash_Client_V1_PostPullRequestCommentRequest()
    request.worktreePath = worktree
    request.number = number
    request.body = sent
    busy = true
    error = nil
    Task { @MainActor in
      defer { busy = false }
      do {
        _ = try await client.postPullRequestComment(request: request, headers: model.headers).result
          .get()
        if comment == sent { comment = "" }
      } catch { self.error = model.failureMessage(error) }
    }
  }

  private func appearanceState(_ state: Releash_Client_V1_PullRequestState) -> String {
    switch state {
    case .draft: "draft"
    case .merged: "merged"
    case .closed: "closed"
    default: "open"
    }
  }

}

struct CheckLogPane: View {
  var model: AppModel
  let worktree: String
  let job: UInt64
  @State private var snapshot = Releash_Client_V1_CheckLogSnapshot()
  @State private var subscription: UUID?

  var body: some View {
    ScrollView([.horizontal, .vertical]) {
      if snapshot.hasReadError { Text(snapshot.readError).foregroundStyle(.red) }
      if snapshot.hasLog {
        Text(snapshot.log).font(.system(.body, design: .monospaced)).textSelection(.enabled)
      }
    }
    .onAppear {
      subscription = model.subscribe("check-log", [worktree, String(job)]) { payload in
        if case .checkLog(let value) = payload.value { snapshot = value }
      }
    }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      subscription = nil
    }
  }
}

private struct PullRequestBullet: StructuredText.UnorderedListMarker {
  func makeBody(configuration: Configuration) -> some View {
    Text("•").textual.frame(minWidth: .fontScaled(1.5), alignment: .trailing)
  }
}
