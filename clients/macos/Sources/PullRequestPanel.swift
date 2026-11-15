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
                Text(pr.title).font(.title3.bold())
                Text("#" + String(pr.number)).font(.title3).foregroundStyle(.secondary)
                Spacer()
                Button {
                  if let url = URL(string: pr.url) { openURL(url) }
                } label: {
                  CardIcon(image: Image("square-arrow-out-up-right"))
                }
                .buttonStyle(IconButtonStyle()).iconButtonFrame()
                .accessibilityLabel("ブラウザで開く")
              }
              StructuredText(markdown: pr.body)
                .environment(
                  \.openURL, OpenURLAction { openCommentLink($0) ? .handled : .discarded })
              if pr.canCreateNew {
                Button("新しい PR を作る") { Task { @MainActor in creating = true } }
              }
              Divider()
              checks(pr.checks)
              Divider()
              HStack {
                Text("COMMENTS").font(.subheadline.bold()).foregroundStyle(.secondary)
                Text("\(pr.comments.count)")
                  .font(.caption).foregroundStyle(.secondary)
                  .padding(.horizontal, 6).padding(.vertical, 2)
                  .background(.quaternary, in: Capsule())
              }
              ForEach(Array(pr.comments.enumerated()), id: \.offset) { _, item in
                HStack(alignment: .top, spacing: UIStyle.spacing) {
                  if item.bot {
                    CardIcon(image: Image("bot"))
                      .frame(width: 24, height: 24)
                      .background(.quaternary, in: Circle()).accessibilityHidden(true)
                  } else {
                    Text(String(item.author.prefix(1)))
                      .frame(width: 24, height: 24).background(.quaternary, in: Circle())
                  }
                  VStack(alignment: .leading, spacing: UIStyle.rowSpacing) {
                    HStack {
                      Text(item.author).bold()
                      if item.bot {
                        Text("bot").font(.caption).foregroundStyle(.secondary)
                          .padding(.horizontal, 4).padding(.vertical, 2)
                          .overlay(
                            RoundedRectangle(cornerRadius: UIStyle.badgeCornerRadius)
                              .strokeBorder(.separator))
                      }
                      TimelineView(.everyMinute) { _ in
                        Text(relativeTimeLabel(Double(item.writtenAt), now: Date()))
                          .foregroundStyle(.secondary)
                      }
                    }
                    StructuredText(markdown: item.body)
                      .environment(
                        \.openURL, OpenURLAction { openCommentLink($0) ? .handled : .discarded })
                  }
                  Spacer(minLength: 0)
                }
                Divider()
              }
            }
          }
          Divider()
          CommentField(
            placeholder: "Comment を書く…（Markdown）", text: $comment,
            identifier: "pr-comment", minimumLines: 3
          ) {
            if !comment.isEmpty && !busy { post(pr.number) }
          }
          .accessibilityLabel("Comment")
          HStack {
            Spacer()
            Button("Comment") { post(pr.number) }.disabled(comment.isEmpty || busy)
              .buttonStyle(.borderedProminent).tint(.blue)
          }
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
      HStack {
        CardIcon(image: Image("git-pull-request")).accessibilityHidden(true)
        Text("このブランチの PR はありません")
      }.foregroundStyle(.secondary)
      Divider()
      Text("Title").foregroundStyle(.secondary)
      TextField("Title", text: $title).accessibilityLabel("Title")
      Text("Description（任意）").foregroundStyle(.secondary)
      TextField("Description（任意）", text: $description, axis: .vertical).lineLimit(3...8)
        .accessibilityLabel("Description（任意）")
      HStack {
        Text("base").foregroundStyle(.secondary).accessibilityHidden(true)
        Picker("base", selection: $base) {
          ForEach(bases.branches, id: \.self) { Text($0).tag($0) }
        }
        .labelsHidden().fixedSize()
        Toggle("draft で作る", isOn: $draft).toggleStyle(.checkbox)
        Spacer(minLength: 0)
        Button("PR を作成", action: create).disabled(title.isEmpty || busy)
          .buttonStyle(.borderedProminent).tint(.blue)
      }
      Divider()
      Spacer()
    }
  }

  private func checks(_ checks: Releash_Client_V1_PullRequestChecks) -> some View {
    VStack(alignment: .leading, spacing: UIStyle.spacing) {
      HStack {
        Text("CHECKS").font(.subheadline.bold()).foregroundStyle(.secondary)
        Spacer()
        if checks.failed > 0 {
          let appearance = UIStyle.checkAppearance(.fail)
          checkCount(checks.failed, label: "失敗", icon: appearance.icon, color: appearance.color)
        }
        if checks.cancelled > 0 {
          let appearance = UIStyle.checkAppearance(.cancel)
          checkCount(
            checks.cancelled, label: "取り消し", icon: appearance.icon, color: appearance.color)
        }
        if checks.pending > 0 {
          let appearance = UIStyle.checkAppearance(.pending)
          checkCount(checks.pending, label: "実行中", icon: appearance.icon, color: appearance.color)
        }
        if checks.skipped > 0 {
          checkCount(checks.skipped, label: "skip", icon: "circle-check", color: .gray)
        }
        if checks.passed > 0 {
          checkCount(checks.passed, label: "通過", icon: "circle-check", color: .green)
        }
      }
      ForEach(Array(checks.listed.enumerated()), id: \.offset) { _, check in
        let appearance = UIStyle.checkAppearance(check.bucket)
        HStack {
          Image(appearance.icon)
            .resizable().renderingMode(.template).scaledToFit().frame(width: 16, height: 16)
            .foregroundStyle(appearance.color)
          Text(check.name)
          if check.hasFailureSeconds {
            HStack(spacing: 4) {
              Text("·").accessibilityHidden(true)
              Text("\(check.failureSeconds / 60) 分で失敗")
            }.foregroundStyle(.secondary)
          }
          if check.hasStartedAt {
            TimelineView(.everyMinute) { _ in
              HStack(spacing: 4) {
                Text("·").accessibilityHidden(true)
                Text(
                  "実行中 \(Int(max(0, Date().timeIntervalSince1970 - Double(check.startedAt))) / 60) 分"
                )
              }.foregroundStyle(.secondary)
            }
          }
          Spacer(minLength: 0)
          if check.hasLogJobID {
            Button("ログを開く") {
              model.panes.openCheckLog(worktree: worktree, job: check.logJobID, title: check.name)
            }
          }
        }
      }
      if checks.passed > 0 {
        Button {
          Task { @MainActor in passedExpanded.toggle() }
        } label: {
          HStack {
            Text(passedExpanded ? "⌄" : "›").accessibilityHidden(true)
            Text("通過した \(checks.passed) 件")
          }
        }.buttonStyle(.plain).foregroundStyle(.secondary)
          .accessibilityLabel("通過した \(checks.passed) 件")
        if passedExpanded {
          ForEach(Array(checks.passedNames.enumerated()), id: \.offset) { _, name in Text(name) }
        }
      }
      if checks.skipped > 0 {
        Button {
          Task { @MainActor in skippedExpanded.toggle() }
        } label: {
          HStack {
            Text(skippedExpanded ? "⌄" : "›").accessibilityHidden(true)
            Text("skip した \(checks.skipped) 件")
          }
        }.buttonStyle(.plain).foregroundStyle(.secondary)
          .accessibilityLabel("skip した \(checks.skipped) 件")
        if skippedExpanded {
          ForEach(Array(checks.skippedNames.enumerated()), id: \.offset) { _, name in Text(name) }
        }
      }
    }
  }

  private func checkCount(_ count: UInt32, label: String, icon: String, color: Color) -> some View {
    HStack(spacing: UIStyle.statusSpacing) {
      CardIcon(image: Image(icon))
      Text("\(count)")
    }
    .foregroundStyle(color)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel("\(label) \(count)")
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
