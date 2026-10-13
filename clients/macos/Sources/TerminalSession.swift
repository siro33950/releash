import AppKit
import Connect
import Observation
import SwiftTerm

@MainActor @Observable final class TerminalSession: NSObject, @preconcurrency TerminalViewDelegate {
  let path: String
  let id: String
  let screen = TerminalScreen(frame: .zero)
  var cwd: String?
  var hasForegroundProcess: Bool?
  var exited = false
  var failure: String?
  @ObservationIgnored private weak var model: AppModel?
  @ObservationIgnored private var subscriber: UUID?
  @ObservationIgnored private var attachment: String?
  @ObservationIgnored private var sequence: UInt64 = 0
  @ObservationIgnored private var pendingInput = ""
  @ObservationIgnored private var processed = 0
  @ObservationIgnored private var reportUnits = 0
  @ObservationIgnored private var replaying = false
  @ObservationIgnored private var applying = false
  @ObservationIgnored private var size = (cols: Int(UIStyle.terminalColumns), rows: Int(UIStyle.terminalRows))
  @ObservationIgnored private let openLink: (URL) -> Void
  @ObservationIgnored private var tail: Task<Void, Never>?

  init(model: AppModel, path: String, id: String, openLink: @escaping (URL) -> Void = { NSWorkspace.shared.open($0) }) {
    self.openLink = openLink
    self.model = model
    self.path = path
    self.id = id
    super.init()
    screen.terminalDelegate = self
    screen.nativeBackgroundColor = .textBackgroundColor
    screen.nativeForegroundColor = .textColor
    screen.setAccessibilityIdentifier("terminal:\(id)")
    subscriber = model.subscribe("workspace-terminal", [path, id]) { [weak self] payload in
      guard case .terminal(let event) = payload.value else { return }
      self?.receive(event)
    }
  }

  var owner: Releash_Client_V1_TerminalSurfaceOwnerV1 {
    var owner = Releash_Client_V1_TerminalSurfaceOwnerV1()
    owner.workspaceTerminal.workspacePath = path
    owner.workspaceTerminal.terminalID = id
    return owner
  }

  func disconnect() {
    attachment = nil
    hasForegroundProcess = nil
    sequence = 0
    processed = 0
  }

  func stop() {
    if let subscriber { model?.unsubscribe(subscriber) }
    subscriber = nil
    disconnect()
    tail?.cancel()
  }

  func receive(_ event: Releash_Client_V1_TerminalEvent) {
    guard let model, let current = model.terminalAttachment(path: path, id: id) else { return }
    applying = true
    defer { applying = false }
    switch event.item {
    case .snapshot(let snapshot):
      if attachment != current { sequence = 0 }
      attachment = current
      processed = 0
      reportUnits = Int(snapshot.processedReportUnits)
      screen.getTerminal().resetToInitialState()
      if !snapshot.replay.isEmpty {
        screen.resize(cols: Int(snapshot.cols), rows: Int(snapshot.rows))
        replaying = true
        screen.feed(text: snapshot.replay)
        replaying = false
      }
      exited = snapshot.isExited
      hasForegroundProcess = snapshot.hasStatus ? snapshot.status.hasForegroundProcess_p : nil
      cwd = snapshot.hasStatus && snapshot.status.hasCwd ? snapshot.status.cwd : nil
      if exited { showExit(snapshot.hasExitCode ? snapshot.exitCode : nil) }
      resize()
      flushInput()
      failure = nil
    case .output(let output):
      guard attachment == current else { return }
      screen.feed(text: output.data)
      processed += output.data.utf16.count
      while reportUnits > 0 && processed >= reportUnits {
        processed -= reportUnits
        var request = Releash_Client_V1_ReportTerminalProcessedRequest()
        request.subscriptionID = current
        request.units = UInt32(reportUnits)
        enqueue(current) { client, headers in
          _ = try await client.reportTerminalProcessed(request: request, headers: headers).result.get()
        }
      }
    case .resize(let resize):
      guard attachment == current else { return }
      screen.resize(cols: Int(resize.cols), rows: Int(resize.rows))
    case .status(let status):
      guard attachment == current else { return }
      cwd = status.hasCwd ? status.cwd : nil
      hasForegroundProcess = status.hasForegroundProcess_p
    case .exit(let exit):
      guard attachment == current else { return }
      exited = true
      hasForegroundProcess = false
      showExit(exit.hasExitCode ? exit.exitCode : nil)
    case nil: failure = "Terminalのイベントがありません"
    }
    screen.needsDisplay = true
  }

  private func showExit(_ code: Int32?) {
    screen.feed(text: "\r\n[Process exited\(code.map { " (code: \($0))" } ?? "")]\r\n")
  }

  func clear() {
    screen.feed(text: "\u{1b}[2J\u{1b}[3J\u{1b}[H")
  }

  func input(_ text: String) {
    guard !exited else { return }
    pendingInput += text
    flushInput()
  }

  private func flushInput() {
    guard let attachment, !pendingInput.isEmpty else { return }
    var request = Releash_Client_V1_WriteTerminalSurfaceRequest()
    request.owner = owner
    request.attachmentID = attachment
    request.sequence = sequence
    request.data = pendingInput
    pendingInput = ""
    sequence += 1
    enqueue(attachment) { client, headers in
      _ = try await client.writeTerminalSurface(request: request, headers: headers).result.get()
    }
  }

  private func enqueue(
    _ attachment: String,
    operation: @escaping @MainActor (any Releash_Client_V1_ClientServiceClientInterface, Connect.Headers) async throws -> Void
  ) {
    let prior = tail
    tail = Task { [weak self] in
      await prior?.value
      guard let self, !Task.isCancelled, self.attachment == attachment,
        let model, let client = model.client else { return }
      do { try await operation(client, model.headers) }
      catch {
        guard self.attachment == attachment else { return }
        failure = model.failureMessage(error)
        disconnect()
        model.restartTerminalSubscription(path: path, id: id)
      }
    }
  }

  private func resize() {
    guard let attachment, !exited else { return }
    var request = Releash_Client_V1_ResizeTerminalSurfaceRequest()
    request.owner = owner
    request.cols = UInt32(size.cols)
    request.rows = UInt32(size.rows)
    enqueue(attachment) { client, headers in
      _ = try await client.resizeTerminalSurface(request: request, headers: headers).result.get()
    }
  }

  func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
    guard !applying, newCols > 0, newRows > 0 else { return }
    size = (newCols, newRows)
    resize()
  }
  func send(source: TerminalView, data: ArraySlice<UInt8>) {
    guard !replaying else { return }
    input(String(decoding: data, as: UTF8.self))
  }
  func clipboardCopy(source: TerminalView, content: Data) {}
  func setTerminalTitle(source: TerminalView, title: String) {}
  func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
  func scrolled(source: TerminalView, position: Double) {}
  func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
  func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
    guard let url = URL(string: link) else { return }
    openLink(url)
  }
}

final class TerminalScreen: TerminalView {
  private var focused = false
  func setFocused(_ value: Bool) {
    let gained = value && !focused
    focused = value
    if gained { window?.makeFirstResponder(self) }
  }
  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    if focused { window?.makeFirstResponder(self) }
  }
  override func isAccessibilityElement() -> Bool { true }
  override func accessibilityRole() -> NSAccessibility.Role? { .textArea }
  override func accessibilityLabel() -> String? { "Terminal" }
  override func accessibilityValue() -> Any? {
    let terminal = getTerminal()
    return (0..<terminal.rows).compactMap { terminal.getLine(row: $0)?.translateToString(trimRight: true, skipNullCellsFollowingWide: true, characterProvider: { terminal.getCharacter(for: $0) }) }.joined(separator: "\n")
  }
}
