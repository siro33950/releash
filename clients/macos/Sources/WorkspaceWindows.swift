import Carbon
import SwiftUI

@MainActor
final class WorkspaceWindows: NSObject, NSApplicationDelegate, NSWindowDelegate {
  let model: AppModel
  let loginItem: LoginItemRegistration
  private(set) var statusMenu: StatusMenu?

  private var windows: [NSWindow] = []
  private var settingsSubscription: UUID?
  private var registrationTask: Task<Void, Never>?
  private var terminationTask: Task<Void, Never>?
  private var awaitingLaunchSettings = false
  var terminationTimeout: Duration = .seconds(5)
  var replyToTermination: (Bool) -> Void = { NSApp.reply(toApplicationShouldTerminate: $0) }

  override convenience init() {
    self.init(model: AppModel())
  }

  init(model: AppModel, loginItem: LoginItemRegistration = LoginItemRegistration()) {
    self.model = model
    self.loginItem = loginItem
    super.init()
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    guard ProcessInfo.processInfo.environment["XCTestConfigurationFilePath"] == nil else { return }
    prepareLaunch(event: NSAppleEventManager.shared().currentAppleEvent)
    Task { await start() }
  }

  func start() async {
    await model.start()
    if awaitingLaunchSettings && model.connectionFailure != nil { showWindow() }
  }

  func prepareLaunch(event: NSAppleEventDescriptor?) {
    guard settingsSubscription == nil else { return }
    statusMenu = StatusMenu(model: model) { [weak self] in self?.showWindow() }
    awaitingLaunchSettings = Self.isLoginItemLaunch(event)
    if !awaitingLaunchSettings { openWindow() }
    settingsSubscription = model.subscribe("desktop-settings") { [weak self] payload in
      guard let self, terminationTask == nil, case .desktopSettings(let settings) = payload.value
      else { return }
      if awaitingLaunchSettings {
        awaitingLaunchSettings = false
        if !settings.autoLaunch || !settings.startMinimized { showWindow() }
      }
      registrationTask?.cancel()
      registrationTask = Task {
        do {
          try await loginItem.synchronize(autoLaunch: settings.autoLaunch, model: model)
        } catch {
          if !Task.isCancelled { model.failure = model.failureMessage(error) }
        }
        if !Task.isCancelled { registrationTask = nil }
      }
    }
  }

  static func isLoginItemLaunch(_ event: NSAppleEventDescriptor?) -> Bool {
    event?.eventID == kAEOpenApplication
      && event?.paramDescriptor(forKeyword: keyAEPropData)?.enumCodeValue
        == keyAELaunchedAsLogInItem
  }

  func showWindow() {
    awaitingLaunchSettings = false
    guard let window = windows.first(where: { $0.isKeyWindow }) ?? windows.last else {
      openWindow()
      return
    }
    if window.isMiniaturized { window.deminiaturize(nil) }
    window.makeKeyAndOrderFront(nil)
    NSApp.activate()
  }

  func openWindow() {
    awaitingLaunchSettings = false
    let window = WorkspaceWindow(
      contentRect: NSRect(x: 0, y: 0, width: UIStyle.windowWidth, height: UIStyle.windowHeight),
      styleMask: [.borderless, .resizable, .miniaturizable, .closable],
      backing: .buffered, defer: false)
    window.title = "Releash"
    window.isReleasedWhenClosed = false
    window.isOpaque = false
    window.backgroundColor = .clear
    window.isMovableByWindowBackground = false
    window.minSize = NSSize(
      width: UIStyle.windowMinimumWidth + UIStyle.windowPadding * 2,
      height: UIStyle.windowMinimumHeight + UIStyle.windowPadding * 2)
    let content = NSHostingView(rootView: RootView(model: model, state: window.state))
    content.safeAreaRegions = []
    window.contentView = content
    window.delegate = self
    if windows.isEmpty {
      if !window.setFrameUsingName("ReleashWorkspace") { window.center() }
      window.setFrameAutosaveName("ReleashWorkspace")
    } else if let previous = windows.last {
      window.cascadeTopLeft(from: NSPoint(x: previous.frame.minX, y: previous.frame.maxY))
    }
    windows.append(window)
    window.makeKeyAndOrderFront(nil)
    NSApp.activate()
  }

  func createBranch() {
    guard let window = NSApp.keyWindow as? WorkspaceWindow, let path = model.selectedWorktree
    else { return }
    window.state.branchDestination = BranchDestination(id: path)
  }

  func applicationShouldHandleReopen(
    _ sender: NSApplication, hasVisibleWindows flag: Bool
  ) -> Bool {
    showWindow()
    return false
  }

  func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    false
  }

  func applicationWillTerminate(_ notification: Notification) {
    statusMenu?.remove()
  }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    if terminationTask != nil { return .terminateLater }
    guard let registrationTask else { return .terminateNow }
    terminationTask = Task {
      do { try await Task.sleep(for: terminationTimeout) } catch { return }
      finishTermination()
    }
    Task {
      await registrationTask.value
      finishTermination()
    }
    return .terminateLater
  }

  private func finishTermination() {
    guard let terminationTask else { return }
    self.terminationTask = nil
    terminationTask.cancel()
    registrationTask?.cancel()
    replyToTermination(true)
  }

  func windowWillClose(_ notification: Notification) {
    guard let window = notification.object as? NSWindow else { return }
    windows.removeAll { $0 === window }
    NSApp.removeWindowsItem(window)
  }
}

final class WorkspaceWindow: NSWindow {
  let state = WorkspaceWindowState()
  override var canBecomeKey: Bool { true }
  override var canBecomeMain: Bool { true }
}

@MainActor @Observable final class WorkspaceWindowState {
  var branchDestination: BranchDestination?
}

struct BranchDestination: Identifiable {
  let id: String
}
