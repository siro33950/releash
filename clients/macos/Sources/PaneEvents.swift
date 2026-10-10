import AppKit
import SwiftUI

struct TabMiddleClick: NSViewRepresentable {
  let close: () -> Void
  func makeNSView(context: Context) -> TabEventView {
    let view = TabEventView()
    view.close = close
    return view
  }
  func updateNSView(_ view: TabEventView, context: Context) { view.close = close }
  static func dismantleNSView(_ view: TabEventView, coordinator: ()) { view.stop() }
}

final class TabEventView: NSView {
  var close: (() -> Void)?
  private var monitor: Any?
  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    stop()
    guard window != nil else { return }
    monitor = NSEvent.addLocalMonitorForEvents(matching: .otherMouseDown) { [weak self] event in
      guard let self, event.buttonNumber == 2, event.window === self.window,
        !self.visibleRect.isEmpty,
        self.visibleRect.contains(self.convert(event.locationInWindow, from: nil))
      else { return event }
      self.close?()
      return nil
    }
  }
  func stop() {
    if let monitor { NSEvent.removeMonitor(monitor) }
    monitor = nil
  }
  override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

struct PaneKeyboardEvents: NSViewRepresentable {
  let model: AppModel
  func makeNSView(context: Context) -> PaneKeyboardView {
    let view = PaneKeyboardView()
    view.model = model
    return view
  }
  func updateNSView(_ view: PaneKeyboardView, context: Context) { view.model = model }
  static func dismantleNSView(_ view: PaneKeyboardView, coordinator: ()) { view.stop() }
}

final class PaneKeyboardView: NSView {
  weak var model: AppModel?
  private var monitor: Any?
  override func viewDidMoveToWindow() {
    super.viewDidMoveToWindow()
    stop()
    guard window != nil else { return }
    monitor = NSEvent.addLocalMonitorForEvents(matching: [.keyDown, .flagsChanged]) {
      [weak self] event in
      guard let self, event.window === self.window, self.window?.attachedSheet == nil,
        let model = self.model, model.selectedWorktree != nil, model.layoutLoaded
      else { return event }
      let flags = event.modifierFlags.intersection([.command, .option, .control, .shift])
      if event.type == .flagsChanged {
        if !flags.contains(.control) { model.finishTabSwitcher() }
        return event
      }
      if !model.paneInteraction.switcherTabs.isEmpty {
        if event.keyCode == 53 {
          model.finishTabSwitcher(cancel: true)
          return nil
        }
        if event.keyCode == 36 {
          model.finishTabSwitcher()
          return nil
        }
        if event.keyCode == 125 {
          model.cycleRecentTab(1)
          return nil
        }
        if event.keyCode == 126 {
          model.cycleRecentTab(-1)
          return nil
        }
      }
      if event.keyCode == 48 && (flags == .control || flags == [.control, .shift]) {
        model.cycleRecentTab(flags.contains(.shift) ? -1 : 1)
        return nil
      }
      if flags == [.command, .option] && (event.keyCode == 123 || event.keyCode == 124) {
        model.cycleTab(event.keyCode == 123 ? -1 : 1)
        return nil
      }
      let key = event.charactersIgnoringModifiers?.lowercased()
      if flags == [.command, .shift] && (key == "[" || key == "]" || key == "{" || key == "}") {
        model.cycleTab(key == "[" || key == "{" ? -1 : 1)
        return nil
      }
      if flags == .command && key == "w", let tab = model.focusedPane?.activeTab {
        model.closeTabs([tab])
        return nil
      }
      if flags == [.command, .option] && key == "t", let pane = model.focusedPane {
        model.closeTabs(pane.tabs.filter { $0.id != pane.activeTab }.map(\.id))
        return nil
      }
      return event
    }
  }
  func stop() {
    if let monitor { NSEvent.removeMonitor(monitor) }
    monitor = nil
  }
  override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

struct PaneTabSwitcher: View {
  @Bindable var model: AppModel
  var body: some View {
    if !model.paneInteraction.switcherTabs.isEmpty {
      VStack(spacing: 0) {
        ForEach(Array(model.paneInteraction.switcherTabs.enumerated()), id: \.element) {
          index, id in
          if let tab = model.layout.tab(id) {
            Button {
              model.paneInteraction.switcherIndex = index
              model.finishTabSwitcher()
            } label: {
              Text(tab.kind.title).frame(maxWidth: .infinity, alignment: .leading)
                .padding(UIStyle.spacing)
                .background(
                  index == model.paneInteraction.switcherIndex
                    ? Color(nsColor: .selectedContentBackgroundColor) : Color.clear
                )
                .foregroundStyle(
                  index == model.paneInteraction.switcherIndex
                    ? Color(nsColor: .selectedTextColor) : Color.primary)
            }.buttonStyle(.plain)
          }
        }
      }
      .padding(UIStyle.spacing)
      .frame(width: UIStyle.switcherWidth)
      .background(.regularMaterial, in: UIStyle.cardShape)
      .overlay(UIStyle.cardShape.stroke(Color(nsColor: .separatorColor)))
      .padding(UIStyle.padding)
    }
  }
}

struct DraggableTabButton: NSViewRepresentable {
  let title: String
  let selected: Bool
  let emphasized: Bool
  let select: () -> Void
  let begin: () -> Void
  let end: () -> Void
  let tabID: String
  let type: String
  func makeNSView(context: Context) -> TabButton {
    let button = TabButton()
    button.isBordered = false
    button.alignment = .right
    button.setButtonType(.momentaryChange)
    button.target = button
    button.action = #selector(TabButton.activateTab)
    return button
  }
  func updateNSView(_ button: TabButton, context: Context) {
    button.title = title
    button.font = .systemFont(ofSize: UIStyle.tabFontSize)
    button.contentTintColor = emphasized ? .labelColor : .secondaryLabelColor
    button.select = select
    button.begin = begin
    button.end = end
    button.tabID = tabID
    button.pasteboardType = NSPasteboard.PasteboardType(type)
    button.setAccessibilityValue(selected)
    button.invalidateIntrinsicContentSize()
  }
}

final class TabButton: NSButton, NSDraggingSource {
  var select: (() -> Void)?
  var begin: (() -> Void)?
  var end: (() -> Void)?
  var tabID = ""
  var pasteboardType = NSPasteboard.PasteboardType.string
  override var intrinsicContentSize: NSSize {
    let width = (title as NSString).size(withAttributes: [
      .font: NSFont.systemFont(ofSize: UIStyle.tabFontSize)
    ]).width
    return NSSize(
      width: ceil(width) + UIStyle.tabLeadingSlot + UIStyle.spacing, height: UIStyle.tabBarHeight)
  }
  @objc func activateTab() { select?() }
  override func mouseDown(with event: NSEvent) {
    guard let window else { return }
    let start = event.locationInWindow
    while let next = window.nextEvent(
      matching: [.leftMouseDragged, .leftMouseUp], until: .distantFuture,
      inMode: .eventTracking, dequeue: true)
    {
      if next.type == .leftMouseUp {
        if bounds.contains(convert(next.locationInWindow, from: nil)) { select?() }
        return
      }
      if hypot(next.locationInWindow.x - start.x, next.locationInWindow.y - start.y)
        < UIStyle.tabDragThreshold
      {
        continue
      }
      let item = NSPasteboardItem()
      item.setString(tabID, forType: pasteboardType)
      let dragging = NSDraggingItem(pasteboardWriter: item)
      let image = NSImage(size: bounds.size)
      image.lockFocus()
      NSColor.controlBackgroundColor.setFill()
      NSRect(origin: .zero, size: bounds.size).fill()
      (title as NSString).draw(
        at: NSPoint(x: UIStyle.spacing, y: UIStyle.spacing),
        withAttributes: [
          .font: NSFont.systemFont(ofSize: UIStyle.tabFontSize),
          .foregroundColor: NSColor.labelColor,
        ])
      image.unlockFocus()
      dragging.setDraggingFrame(bounds, contents: image)
      begin?()
      beginDraggingSession(with: [dragging], event: next, source: self)
      return
    }
  }
  func draggingSession(
    _ session: NSDraggingSession, sourceOperationMaskFor context: NSDraggingContext
  ) -> NSDragOperation { context == .withinApplication ? [.move, .copy] : [] }
  func draggingSession(
    _ session: NSDraggingSession, endedAt screenPoint: NSPoint, operation: NSDragOperation
  ) { end?() }
}
