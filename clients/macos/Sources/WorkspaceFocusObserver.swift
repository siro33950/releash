import AppKit
import SwiftUI

struct WorkspaceFocusObserver: NSViewRepresentable {
  let onFocus: () -> Void

  func makeNSView(context: Context) -> ObserverView { ObserverView(onFocus: onFocus) }
  func updateNSView(_ view: ObserverView, context: Context) { view.onFocus = onFocus }

  func sizeThatFits(_ proposal: ProposedViewSize, nsView: ObserverView, context: Context) -> CGSize?
  {
    proposal.replacingUnspecifiedDimensions()
  }

  static func dismantleNSView(_ view: ObserverView, coordinator: ()) { view.stopMonitoring() }

  final class ObserverView: NSView {
    var onFocus: () -> Void
    private var monitor: Any?

    init(onFocus: @escaping () -> Void) {
      self.onFocus = onFocus
      super.init(frame: .zero)
    }
    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override func viewDidMoveToWindow() {
      super.viewDidMoveToWindow()
      stopMonitoring()
      guard window != nil else { return }
      monitor = NSEvent.addLocalMonitorForEvents(matching: .leftMouseDown) { [weak self] event in
        if let self, event.window === self.window,
          self.bounds.contains(self.convert(event.locationInWindow, from: nil))
        {
          self.onFocus()
        }
        return event
      }
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }

    func stopMonitoring() {
      if let monitor { NSEvent.removeMonitor(monitor) }
      monitor = nil
    }
  }
}
