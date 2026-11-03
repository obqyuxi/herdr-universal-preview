// Generates synthetic, non-sensitive test media with system frameworks only.
//   swiftc -O media.swift -o media && ./media <out-dir>
// - sample-3pages.pdf  : 3 pages, real text layer (English + 中文), drawn with CoreText
// - sample-h264-aac.mp4 / sample-h264-aac.mov : 6 s, 640x360 H.264 + 44.1 kHz AAC stereo tone
import AVFoundation
import CoreGraphics
import CoreText
import Foundation

let out = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ".")

func drawText(_ ctx: CGContext, _ s: String, x: CGFloat, y: CGFloat, size: CGFloat) {
    let font = CTFontCreateWithName("PingFang SC" as CFString, size, nil)
    let attr = NSAttributedString(string: s, attributes: [kCTFontAttributeName as NSAttributedString.Key: font])
    let line = CTLineCreateWithAttributedString(attr)
    ctx.textPosition = CGPoint(x: x, y: y)
    CTLineDraw(line, ctx)
}

func makePDF() {
    var box = CGRect(x: 0, y: 0, width: 595, height: 842)
    let url = out.appendingPathComponent("sample-3pages.pdf") as CFURL
    guard let ctx = CGContext(url, mediaBox: &box, nil) else { fatalError("pdf context") }
    for page in 1...3 {
        ctx.beginPDFPage(nil)
        ctx.setFillColor(CGColor(red: 0.93, green: 0.95, blue: 1, alpha: 1))
        ctx.fill(CGRect(x: 40, y: 700, width: 515, height: 100))
        drawText(ctx, "Herdr Universal Preview — page \(page) of 3", x: 60, y: 760, size: 22)
        drawText(ctx, "中文文本层测试：第 \(page) 页，可以选中并复制。", x: 60, y: 720, size: 18)
        for i in 0..<20 {
            drawText(ctx, "Line \(i + 1) on page \(page): The quick brown fox jumps over the lazy dog. 敏捷的棕色狐狸。", x: 60, y: 660 - CGFloat(i) * 28, size: 12)
        }
        ctx.endPDFPage()
    }
    ctx.closePDF()
}

func makeVideo(_ name: String, _ type: AVFileType) throws {
    let url = out.appendingPathComponent(name)
    try? FileManager.default.removeItem(at: url)
    let w = 640, h = 360, fps: Int32 = 30, seconds = 6
    let writer = try AVAssetWriter(outputURL: url, fileType: type)
    let vIn = AVAssetWriterInput(mediaType: .video, outputSettings: [
        AVVideoCodecKey: AVVideoCodecType.h264, AVVideoWidthKey: w, AVVideoHeightKey: h,
    ])
    let adaptor = AVAssetWriterInputPixelBufferAdaptor(assetWriterInput: vIn, sourcePixelBufferAttributes: [
        kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA, kCVPixelBufferWidthKey as String: w, kCVPixelBufferHeightKey as String: h,
    ])
    let rate = 44100.0
    let aIn = AVAssetWriterInput(mediaType: .audio, outputSettings: [
        AVFormatIDKey: kAudioFormatMPEG4AAC, AVSampleRateKey: rate, AVNumberOfChannelsKey: 2, AVEncoderBitRateKey: 128000,
    ])
    vIn.expectsMediaDataInRealTime = false
    aIn.expectsMediaDataInRealTime = false
    writer.add(vIn); writer.add(aIn)
    guard writer.startWriting() else { throw writer.error ?? NSError(domain: "writer", code: 1) }
    writer.startSession(atSourceTime: .zero)

    // Both inputs are fed from their own queues as the writer asks, so it can
    // interleave them (feeding all video first deadlocks a non-realtime writer).
    let group = DispatchGroup()
    var frame = 0
    let frames = Int(fps) * seconds
    group.enter()
    vIn.requestMediaDataWhenReady(on: DispatchQueue(label: "v")) {
        while vIn.isReadyForMoreMediaData {
            if frame >= frames { vIn.markAsFinished(); group.leave(); return }
            let f = frame; frame += 1
            var pb: CVPixelBuffer?
            CVPixelBufferPoolCreatePixelBuffer(nil, adaptor.pixelBufferPool!, &pb)
            guard let buf = pb else { fatalError("pixel buffer") }
            CVPixelBufferLockBaseAddress(buf, [])
            let ctx = CGContext(data: CVPixelBufferGetBaseAddress(buf), width: w, height: h, bitsPerComponent: 8,
                                bytesPerRow: CVPixelBufferGetBytesPerRow(buf), space: CGColorSpaceCreateDeviceRGB(),
                                bitmapInfo: CGImageAlphaInfo.premultipliedFirst.rawValue | CGBitmapInfo.byteOrder32Little.rawValue)!
            // Background hue and a row of blocks show the second; a bar sweeps each second.
            let sec = f / Int(fps)
            let hue = CGFloat(sec) / CGFloat(seconds)
            ctx.setFillColor(CGColor(red: 0.1 + 0.8 * hue, green: 0.3, blue: 0.9 - 0.7 * hue, alpha: 1))
            ctx.fill(CGRect(x: 0, y: 0, width: w, height: h))
            ctx.setFillColor(CGColor(gray: 1, alpha: 1))
            ctx.fill(CGRect(x: CGFloat(f % Int(fps)) * CGFloat(w) / CGFloat(fps), y: 20, width: 20, height: 40))
            for i in 0...sec { ctx.fill(CGRect(x: 40 + i * 60, y: 200, width: 40, height: 100)) }
            CVPixelBufferUnlockBaseAddress(buf, [])
            adaptor.append(buf, withPresentationTime: CMTime(value: CMTimeValue(f), timescale: fps))
        }
    }

    // Audio: a tone whose pitch rises each second (220 Hz, 330 Hz, ...), stereo float -> AAC.
    let total = Int(rate) * seconds, chunk = 4096
    var written = 0
    var asbd = AudioStreamBasicDescription(mSampleRate: rate, mFormatID: kAudioFormatLinearPCM,
        mFormatFlags: kAudioFormatFlagIsFloat | kAudioFormatFlagIsPacked, mBytesPerPacket: 8, mFramesPerPacket: 1,
        mBytesPerFrame: 8, mChannelsPerFrame: 2, mBitsPerChannel: 32, mReserved: 0)
    var fmt: CMAudioFormatDescription?
    CMAudioFormatDescriptionCreate(allocator: nil, asbd: &asbd, layoutSize: 0, layout: nil, magicCookieSize: 0, magicCookie: nil, extensions: nil, formatDescriptionOut: &fmt)
    group.enter()
    aIn.requestMediaDataWhenReady(on: DispatchQueue(label: "a")) {
        while aIn.isReadyForMoreMediaData {
            if written >= total { aIn.markAsFinished(); group.leave(); return }
            let n = min(chunk, total - written)
            var samples = [Float](repeating: 0, count: n * 2)
            for i in 0..<n {
                let t = Double(written + i) / rate
                let v = Float(0.25 * sin(2 * .pi * (220 + 110 * floor(t)) * t))
                samples[2 * i] = v; samples[2 * i + 1] = v
            }
            var block: CMBlockBuffer?
            let bytes = n * 8
            CMBlockBufferCreateWithMemoryBlock(allocator: nil, memoryBlock: nil, blockLength: bytes, blockAllocator: nil, customBlockSource: nil, offsetToData: 0, dataLength: bytes, flags: 0, blockBufferOut: &block)
            _ = samples.withUnsafeBytes { CMBlockBufferReplaceDataBytes(with: $0.baseAddress!, blockBuffer: block!, offsetIntoDestination: 0, dataLength: bytes) }
            var sb: CMSampleBuffer?
            CMAudioSampleBufferCreateReadyWithPacketDescriptions(allocator: nil, dataBuffer: block!, formatDescription: fmt!, sampleCount: n, presentationTimeStamp: CMTime(value: CMTimeValue(written), timescale: CMTimeScale(rate)), packetDescriptions: nil, sampleBufferOut: &sb)
            aIn.append(sb!)
            written += n
        }
    }
    group.wait()
    let done = DispatchSemaphore(value: 0)
    writer.finishWriting { done.signal() }
    done.wait()
    if writer.status != .completed { throw writer.error ?? NSError(domain: "writer", code: 2) }
}

makePDF()
try makeVideo("sample-h264-aac.mp4", .mp4)
try makeVideo("sample-h264-aac.mov", .mov)
print("ok")
