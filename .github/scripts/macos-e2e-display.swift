import CoreGraphics
import Foundation

let display = CGMainDisplayID()
guard let modes = CGDisplayCopyAllDisplayModes(display, nil) as? [CGDisplayMode],
  let mode = modes.first(where: { $0.width == 1920 && $0.height == 1080 })
else { fatalError("The E2E display does not offer 1920x1080") }
var configuration: CGDisplayConfigRef?
guard CGBeginDisplayConfiguration(&configuration) == .success,
  let configuration
else { fatalError("Cannot configure the E2E display") }
guard CGConfigureDisplayWithDisplayMode(configuration, display, mode, nil) == .success else {
  CGCancelDisplayConfiguration(configuration)
  fatalError("Cannot select 1920x1080")
}
guard CGCompleteDisplayConfiguration(configuration, .forSession) == .success,
  let actual = CGDisplayCopyDisplayMode(display), actual.width == 1920, actual.height == 1080
else { fatalError("E2E display resolution was not applied") }
print("E2E display: \(actual.width)x\(actual.height)")
