import Bonsplit
import Observation
import SwiftUI
import WebKit

struct FileTabPane: View {
  var model: AppModel
  let tabID: TabID
  let path: String
  let worktree: String
  @Environment(\.colorScheme) private var colorScheme
  var settings = FileDisplaySettings()
  @State var value: Releash_Client_V1_FileTabView?
  @State var threads = Releash_Client_V1_FileThreadsSnapshot()
  @State private var subscription: UUID?
  @State private var threadSubscription: UUID?
  @State var draft = FileThreadDraft()
  var tab: PaneTab? { model.filePanes.tab(tabID) }
  var base: String { tab?.fileBase ?? "head" }
  var mode: String { tab?.fileMode ?? value?.initialMode ?? "file" }

  var body: some View {
    VStack(spacing: 0) {
      toolbar
      Divider()
      if let value, let view = value.view.variant {
        switch view {
        case .textDiff(let text):
          FileWebView(
            payload: bridgePayload(text, value, colorScheme: colorScheme), model: model,
            onAction: { message, accepted in action(message, onReplyAccepted: accepted) }
          )
          .accessibilityIdentifier("file-content")
        case .image(let image):
          ScrollView([.horizontal, .vertical]) {
            HStack {
              ForEach(
                Array(imageURLs(image, initialMode: value.initialMode).enumerated()), id: \.offset
              ) { item in
                imageView(item.element.url, label: item.element.label)
              }
            }
          }
        case .binary(let binary):
          VStack {
            ForEach(binaryDescription(binary), id: \.self) { Text($0) }
            Button("外部エディタで開く", action: openInEditor)
          }.frame(maxWidth: .infinity, maxHeight: .infinity)
        }
      } else {
        ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
      }
    }
    .onAppear {
      subscribe()
      threadSubscription = model.subscribe("file-threads", [worktree, path]) { payload in
        if case .fileThreads(let value) = payload.value { threads = value }
      }
    }
    .onDisappear {
      if let subscription { model.unsubscribe(subscription) }
      if let threadSubscription { model.unsubscribe(threadSubscription) }
    }
    .onChange(of: base) { _, _ in subscribe() }

  }
  var toolbar: some View {
    HStack(spacing: 4) {
      Text(path).accessibilityIdentifier("file-path").lineLimit(1).truncationMode(.middle)
        .layoutPriority(-1).help(path)
      Spacer(minLength: 8)
      if draft.selection != nil {
        Button("Thread作成", action: startThread).fixedSize().accessibilityIdentifier(
          "file-create-thread")
      }
      if mode == "diff" {
        IconChoice(
          selection: Binding(get: { base }, set: { setPresentation(base: $0) }),
          options: [
            ("head", "HEAD と比較", "git-commit-horizontal"),
            ("branch-base", "base と比較", "git-branch"),
          ]
        ).accessibilityIdentifier("file-base")
        IconChoice(
          selection: settings.$style,
          options: [
            ("unified", "上下に並べる", "rows-2"),
            ("split", "左右に並べる", "columns-2"),
          ]
        ).accessibilityIdentifier("file-style")
        Toggle(isOn: settings.$full) {
          CardLabel(title: "全文", image: "unfold-vertical")
        }
        .toggleStyle(IconToggleStyle()).labelStyle(.iconOnly).iconButtonFrame().help("全文")
        .accessibilityIdentifier("file-full")
      }
      Toggle(
        isOn: Binding(get: { mode == "diff" }, set: { setPresentation(mode: $0 ? "diff" : "file") })
      ) {
        CardLabel(title: "Diff", image: "diff")
      }
      .toggleStyle(IconToggleStyle()).labelStyle(.iconOnly).iconButtonFrame().help("Diff 表示")
      .accessibilityIdentifier("file-mode")
      Button(action: openInEditor) {
        CardLabel(title: "外部エディタで開く", image: "square-arrow-out-up-right")
      }
      .labelStyle(.iconOnly).buttonStyle(IconButtonStyle()).iconButtonFrame().help("外部エディタで開く")
      .accessibilityIdentifier("file-open-editor")
    }.padding(8).frame(height: 44)
  }
  /// アプリのアクセントカラーを、外観に合わせて解決した 16 進の文字列にする。viewer の行選択と Thread の範囲の印が使う
  static func accentHex(_ colorScheme: ColorScheme) -> String {
    var hex = "#007AFF"
    NSAppearance(named: colorScheme == .dark ? .darkAqua : .aqua)?.performAsCurrentDrawingAppearance
    {
      guard let color = NSColor.controlAccentColor.usingColorSpace(.sRGB) else { return }
      hex = String(
        format: "#%02X%02X%02X", Int((color.redComponent * 255).rounded()),
        Int((color.greenComponent * 255).rounded()), Int((color.blueComponent * 255).rounded()))
    }
    return hex
  }
  func subscribe() {
    if let subscription { model.unsubscribe(subscription) }
    value = nil
    subscription = model.subscribe("file-tab", [worktree, path, base]) { payload in
      guard case .fileTab(let received) = payload.value else { return }
      value = received
      if tab?.fileMode == nil { setPresentation(mode: received.initialMode) }
    }
  }
  func setPresentation(mode: String? = nil, base: String? = nil) {
    model.filePanes.setFilePresentation(tabID, mode: mode, base: base)
  }
  func openInEditor() {
    guard let client = model.client else { return }
    var request = Releash_Client_V1_OpenInEditorRequest()
    request.worktreePath = worktree
    request.file = path
    model.perform {
      _ = try await client.openInEditor(request: request, headers: model.headers).result.get()
    }
  }
  func imageURLs(_ image: Releash_Client_V1_ReviewImageDto, initialMode: String) -> [(
    url: String, label: String
  )] {
    var images: [(url: String, label: String)] = []
    if mode == "diff", initialMode == "diff", image.hasOriginalURL {
      images.append((image.originalURL, "変更前の画像"))
    }
    if image.hasModifiedURL { images.append((image.modifiedURL, "作業ツリーの画像")) }
    return images
  }
  func binaryDescription(_ binary: Releash_Client_V1_ReviewBinaryDto) -> [String] {
    var text = ["画像以外のバイナリは表示できません"]
    if binary.hasModifiedSize {
      text.append("\(binary.modifiedSize) bytes")
    } else if binary.hasOriginalSize {
      text.append("\(binary.originalSize) bytes")
    }
    return text
  }
  @ViewBuilder func imageView(_ url: String, label: String) -> some View {
    if let comma = url.firstIndex(of: ","),
      let data = Data(base64Encoded: String(url[url.index(after: comma)...])),
      let image = NSImage(data: data)
    {
      Image(nsImage: image).resizable().scaledToFit().frame(width: min(image.size.width, 600))
        .accessibilityLabel(label)
    } else {
      Text("画像を表示できません")
    }
  }
  func bridgePayload(
    _ text: Releash_Client_V1_ReviewTextDiffDto, _ value: Releash_Client_V1_FileTabView,
    colorScheme: ColorScheme
  ) -> [String: Any] {
    var result: [String: Any] = [
      "path": path, "original": text.original,
      "modified": text.modified,
      "patch": value.patch, "deleted": text.source.value == .deleted, "mode": mode, "base": base,
      "style": settings.style, "full": settings.full, "dark": colorScheme == .dark,
      "accent": Self.accentHex(colorScheme),
      "threads": threads.threads.items.compactMap { thread -> [String: Any]? in
        guard thread.target.hasLineNumber else { return nil }
        var annotation: [String: Any] = [
          "id": thread.id, "line": thread.target.lineNumber,
          "resolved": thread.state.value == .resolved,
          "canResolve": thread.canResolve,
          "resolvedBy": thread.resolve.actor.displayName,
          "resolvedAt": thread.resolve.resolvedAt,
          "comments": thread.comments.items.map { comment -> [String: Any] in
            var value: [String: Any] = [
              "author": comment.author.displayName, "content": comment.content,
              "agent": comment.author.kind.value == .agent, "createdAt": comment.createdAt,
            ]
            if comment.author.kind.value == .agent, comment.author.hasModel {
              value["model"] = comment.author.model
            }
            return value
          },
        ]
        if thread.target.hasEndLine { annotation["endLine"] = thread.target.endLine }
        if thread.target.hasSide {
          annotation["side"] = thread.target.side.value == .deletions ? "deletions" : "additions"
        }
        if thread.target.hasEndSide {
          annotation["endSide"] =
            thread.target.endSide.value == .deletions ? "deletions" : "additions"
        }
        return annotation
      },
    ]
    if let line = draft.line {
      var composer: [String: Any] = [
        "line": line, "endLine": draft.endLine ?? line, "id": draft.id.uuidString,
        "sending": draft.sending,
      ]
      if let side = draft.side { composer["side"] = side == .deletions ? "deletions" : "additions" }
      if let side = draft.endSide {
        composer["endSide"] = side == .deletions ? "deletions" : "additions"
      }
      result["composer"] = composer
    }
    return result
  }
  func action(_ message: [String: Any], onReplyAccepted: @escaping () -> Void = {}) {
    guard let kind = message["kind"] as? String, let client = model.client else { return }
    if kind == "select" {
      draft.selection = FileLineSelection(message)
      return
    }
    if kind == "cancelCreate" {
      guard !draft.sending else { return }
      draft.line = nil
      return
    }
    if kind == "create" {
      draft.content = message["content"] as? String ?? ""
      createThread()
      return
    }
    guard let id = message["id"] as? String else { return }
    model.perform {
      switch kind {
      case "reply":
        var request = Releash_Client_V1_AppendReviewCommentRequest()
        request.worktreeName = worktree
        request.threadID = id
        request.content = message["content"] as? String ?? ""
        _ = try await client.appendReviewComment(request: request, headers: model.headers).result
          .get()
        onReplyAccepted()
      case "resolve":
        var request = Releash_Client_V1_ResolveReviewThreadRequest()
        request.worktreeName = worktree
        request.threadID = id
        request.outcome = "resolved"
        request.summary = "ファイルタブで解決"
        _ = try await client.resolveReviewThread(request: request, headers: model.headers).result
          .get()
      default: break
      }
    }
  }
  func startThread() {
    guard let selection = draft.selection else { return }
    guard !draft.sending else { return }
    draft.id = UUID()
    draft.line = selection.line
    draft.endLine = selection.endLine
    draft.side = selection.side
    draft.endSide = selection.endSide
    draft.content = ""
  }
  func createThread() {
    guard let client = model.client, let line = draft.line, !draft.sending else { return }
    guard !draft.content.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
    var request = Releash_Client_V1_CreateReviewThreadRequest()
    request.worktreeName = worktree
    request.filePath = path
    request.lineNumber = line
    if let endLine = draft.endLine { request.endLine = endLine }
    if let side = draft.side { request.side.value = side }
    if let endSide = draft.endSide { request.endSide.value = endSide }
    request.content = draft.content
    let id = draft.id
    draft.sending = true
    model.perform {
      defer { draft.sending = false }
      _ = try await client.createReviewThread(request: request, headers: model.headers).result.get()
      if draft.id == id { draft.line = nil }
    }
  }
}

struct FileDisplaySettings: DynamicProperty {
  @AppStorage("diffStyle") var style = "split"
  @AppStorage("diffFull") var full = false
}

struct FileLineSelection: Equatable {
  var line: UInt32
  var endLine: UInt32
  var side: Releash_Client_V1_ReviewLineSideDto.Value?
  var endSide: Releash_Client_V1_ReviewLineSideDto.Value?
  init?(_ message: [String: Any]) {
    guard let line = (message["line"] as? NSNumber)?.uint32Value else { return nil }
    self.line = line
    endLine = (message["endLine"] as? NSNumber)?.uint32Value ?? line
    let side = { (value: Any?) -> Releash_Client_V1_ReviewLineSideDto.Value? in
      switch value as? String {
      case "deletions": .deletions
      case "additions": .additions
      default: nil
      }
    }
    self.side = side(message["side"])
    endSide = side(message["endSide"])
  }
}

@Observable @MainActor final class FileThreadDraft {
  var id = UUID()
  var sending = false
  var selection: FileLineSelection?
  var line: UInt32?
  var endLine: UInt32?
  var side: Releash_Client_V1_ReviewLineSideDto.Value?
  var endSide: Releash_Client_V1_ReviewLineSideDto.Value?
  var content = ""
}

struct FileWebView: NSViewRepresentable {
  let payload: [String: Any]
  let model: AppModel
  let onAction: ([String: Any], @escaping () -> Void) -> Void
  func makeCoordinator() -> Coordinator { Coordinator(model: model, onAction: onAction) }
  func makeNSView(context: Context) -> WKWebView {
    let configuration = WKWebViewConfiguration()
    let handler = context.coordinator
    configuration.userContentController.add(
      handler, contentWorld: handler.world, name: "fileViewer")
    let web = WKWebView(frame: .zero, configuration: configuration)
    // 既定では WKWebView がページを描くまで白を塗る。切って、描くまでは後ろの面の色を見せる
    web.setValue(false, forKey: "drawsBackground")
    handler.web = web
    web.navigationDelegate = handler
    do {
      guard let folder = Bundle.main.url(forResource: "FileViewer", withExtension: nil) else {
        throw CLIFailure(message: "Bundled file viewer is missing")
      }
      let script = try String(
        contentsOf: folder.appendingPathComponent("viewer.js"), encoding: .utf8)
      configuration.userContentController.addUserScript(
        WKUserScript(
          source: script, injectionTime: .atDocumentEnd, forMainFrameOnly: true, in: handler.world))
      web.loadFileURL(folder.appendingPathComponent("index.html"), allowingReadAccessTo: folder)
    } catch { model.failure = error.localizedDescription }
    return web
  }
  func updateNSView(_ web: WKWebView, context: Context) {
    context.coordinator.payload = payload
    context.coordinator.onAction = onAction
    context.coordinator.render()
  }
  static func dismantleNSView(_ web: WKWebView, coordinator: Coordinator) {
    web.configuration.userContentController.removeScriptMessageHandler(
      forName: "fileViewer", contentWorld: coordinator.world)
  }
  @MainActor final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
    let world = WKContentWorld.world(name: "ReleashFileViewer")
    let model: AppModel
    weak var web: WKWebView?
    var payload: [String: Any] = [:]
    var ready = false
    private var composing = false
    var onAction: ([String: Any], @escaping () -> Void) -> Void
    init(model: AppModel, onAction: @escaping ([String: Any], @escaping () -> Void) -> Void) {
      self.model = model
      self.onAction = onAction
    }
    func userContentController(
      _ userContentController: WKUserContentController, didReceive message: WKScriptMessage
    ) {
      guard let value = message.body as? [String: Any] else { return }
      if value["kind"] as? String == "ready" {
        ready = true
        render()
      } else {
        onAction(value) { [weak self] in
          guard let self, let id = value["id"] as? String,
            let requestID = value["requestID"] as? NSNumber
          else { return }
          web?.callAsyncJavaScript(
            "window.releashReplyAccepted(id, requestID)",
            arguments: ["id": id, "requestID": requestID], in: nil, in: world
          ) { [weak self] result in
            if case .failure(let error) = result {
              self?.model.failure = error.localizedDescription
            }
          }
        }
      }
    }
    func render() {
      guard ready, !payload.isEmpty else { return }
      if payload["composer"] != nil, !composing {
        web?.window?.makeFirstResponder(web)
      }
      composing = payload["composer"] != nil
      web?.callAsyncJavaScript(
        "window.releashRender(payload)", arguments: ["payload": payload], in: nil, in: world
      ) { [weak self] result in
        if case .failure(let error) = result { self?.model.failure = error.localizedDescription }
      }
    }
    func webView(
      _ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction,
      decisionHandler: @escaping @MainActor @Sendable (WKNavigationActionPolicy) -> Void
    ) {
      if navigationAction.navigationType == .linkActivated {
        if let url = navigationAction.request.url,
          ["https", "http", "mailto"].contains(url.scheme?.lowercased() ?? "")
        {
          NSWorkspace.shared.open(url)
        }
        decisionHandler(.cancel)
        return
      }
      decisionHandler(navigationAction.request.url?.isFileURL == true ? .allow : .cancel)
    }
  }
}

/// アイコンだけの択一。選んだ項目は IconToggleStyle の on と同じ色で出る
struct IconChoice<Value: Hashable>: View {
  @Binding var selection: Value
  let options: [(value: Value, title: String, symbol: String)]
  var body: some View {
    HStack(spacing: 0) {
      ForEach(options, id: \.value) { option in
        Toggle(
          isOn: Binding(
            get: { selection == option.value }, set: { if $0 { selection = option.value } })
        ) {
          CardLabel(title: option.title, image: option.symbol)
        }
        .toggleStyle(IconToggleStyle()).labelStyle(.iconOnly)
        .iconButtonFrame()
        .help(option.title).accessibilityLabel(option.title)
      }
    }
    .accessibilityElement(children: .contain)
  }
}
