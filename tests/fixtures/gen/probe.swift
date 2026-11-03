// Prints each media file's container, tracks, codecs and duration (for SUPPORT_MATRIX).
//   swiftc -O probe.swift -o probe && ./probe file...
import AVFoundation
func fourcc(_ c: FourCharCode) -> String { String(bytes: [24, 16, 8, 0].map { UInt8((c >> $0) & 0xff) }, encoding: .ascii) ?? "\(c)" }
for path in CommandLine.arguments.dropFirst() {
    let asset = AVURLAsset(url: URL(fileURLWithPath: path))
    let sem = DispatchSemaphore(value: 0)
    Task {
        do {
            let d = try await asset.load(.duration)
            var parts: [String] = []
            for t in try await asset.load(.tracks) {
                let descs = try await t.load(.formatDescriptions)
                let codec = descs.first.map { fourcc(CMFormatDescriptionGetMediaSubType($0)) } ?? "?"
                var extra = ""
                if t.mediaType == .video { let s = try await t.load(.naturalSize); extra = " \(Int(s.width))x\(Int(s.height))" }
                parts.append("\(t.mediaType.rawValue):\(codec)\(extra)")
            }
            print("\((path as NSString).lastPathComponent)\tduration=\(String(format: "%.2f", d.seconds))s\t\(parts.joined(separator: " "))")
        } catch { print("\((path as NSString).lastPathComponent)\terror=\(error.localizedDescription)") }
        sem.signal()
    }
    sem.wait()
}
