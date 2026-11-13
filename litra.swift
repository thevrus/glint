// litra: control a Logitech Litra Beam LX over BLE HID (HID++ 2.0), no Logitech software.
// Usage: litra status | on | off | toggle | brightness <lumens> | temp <kelvin>
import IOKit.hid
import Foundation

let m = IOHIDManagerCreate(kCFAllocatorDefault, IOOptionBits(kIOHIDOptionsTypeNone))
IOHIDManagerSetDeviceMatching(m, [kIOHIDVendorIDKey: 0x046d] as CFDictionary)
IOHIDManagerOpen(m, IOOptionBits(kIOHIDOptionsTypeNone))
let litras = ((IOHIDManagerCopyDevices(m) as? Set<IOHIDDevice>) ?? []).filter {
  (IOHIDDeviceGetProperty($0, kIOHIDProductKey as CFString) as? String ?? "").contains("Litra")
}
guard let dev = litras.first else { fputs("No Litra found (is it paired/connected?)\n", stderr); exit(1) }
IOHIDDeviceOpen(dev, IOOptionBits(kIOHIDOptionsTypeNone))

var last: [UInt8] = []
let buf = UnsafeMutablePointer<UInt8>.allocate(capacity: 64)
IOHIDDeviceRegisterInputReportCallback(dev, buf, 64, { _, _, _, _, _, rep, len in
  last = (0..<len).map { rep[$0] }
}, nil)
IOHIDDeviceScheduleWithRunLoop(dev, CFRunLoopGetMain(), CFRunLoopMode.defaultMode.rawValue)

func raw(_ idx: UInt8, _ fnSw: UInt8, _ args: [UInt8]) -> [UInt8]? {
  var p: [UInt8] = [0x11, 0xff, idx, fnSw] + args
  p += [UInt8](repeating: 0, count: 20 - p.count)
  last = []
  guard IOHIDDeviceSetReport(dev, kIOHIDReportTypeOutput, 0x11, &p, p.count) == 0 else { return nil }
  let end = Date().addingTimeInterval(1)
  while last.isEmpty && Date() < end { RunLoop.main.run(until: Date().addingTimeInterval(0.02)) }
  guard last.count >= 4, last[2] == idx else { return nil }   // 0xff in [2] = HID++ error
  return Array(last[4...])
}

// IRoot.GetFeature(0x1990 Illumination) -> feature index (differs between USB and BLE firmware)
let lightIdx = raw(0x00, 0x0a, [0x19, 0x90])?.first ?? 0
guard lightIdx != 0 else { fputs("Illumination feature not found\n", stderr); exit(1) }

@discardableResult
func call(_ fnSw: UInt8, _ args: [UInt8] = []) -> [UInt8]? { raw(lightIdx, fnSw, args) }

func status() {
  let on = call(0x01)?.first == 1
  let br = call(0x31).map { Int($0[0]) << 8 | Int($0[1]) } ?? -1
  let k = call(0x81).map { Int($0[0]) << 8 | Int($0[1]) } ?? -1
  print("power: \(on ? "on" : "off")  brightness: \(br) lm  temp: \(k) K")
}

let a = CommandLine.arguments.dropFirst()
switch a.first {
case "on": call(0x1c, [1]); status()
case "off": call(0x1c, [0]); status()
case "toggle": call(0x1c, [call(0x01)?.first == 1 ? 0 : 1]); status()
case "brightness":
  guard let v = a.dropFirst().first.flatMap({ Int($0) }) else { fputs("brightness <lumens>\n", stderr); exit(2) }
  if call(0x4c, [UInt8(v >> 8), UInt8(v & 0xff)]) == nil { fputs("rejected (out of range?)\n", stderr) }
  status()
case "temp":
  guard let v = a.dropFirst().first.flatMap({ Int($0) }) else { fputs("temp <kelvin>\n", stderr); exit(2) }
  if call(0x9c, [UInt8(v >> 8), UInt8(v & 0xff)]) == nil { fputs("rejected (out of range?)\n", stderr) }
  status()
case "info":
  print("brightness info:", call(0x21) ?? [], " temp info:", call(0x71) ?? [])
default: status()
}
