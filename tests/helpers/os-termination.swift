import Foundation
import Carbon

let pid = pid_t(CommandLine.arguments[1])!
let reasons = ["logout": kAEReallyLogOut, "system-restart": kAERestart, "shutdown": kAEShutDown]
let reason = reasons[CommandLine.arguments[2]]!
let event = NSAppleEventDescriptor(eventClass: AEEventClass(kCoreEventClass), eventID: AEEventID(kAEQuitApplication), targetDescriptor: NSAppleEventDescriptor(processIdentifier: pid), returnID: AEReturnID(kAutoGenerateReturnID), transactionID: AETransactionID(kAnyTransactionID))
event.setAttribute(NSAppleEventDescriptor(enumCode: OSType(reason)), forKeyword: AEKeyword(kAEQuitReason))
let reply = try event.sendEvent(options: [.waitForReply, .neverInteract], timeout: 30)
if let error = reply.paramDescriptor(forKeyword: AEKeyword(keyErrorNumber)), error.int32Value != 0 {
    fatalError("OS termination was rejected: \(error.int32Value)")
}
