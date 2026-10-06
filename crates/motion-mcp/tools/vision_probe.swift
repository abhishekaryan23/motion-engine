// (0.21) On-device image probe with Apple Vision (macOS 12+), for the MCP
// server's bring-your-own-images step (plan §5a). No network, no model
// download. Compiled once by `motion-mcp` into `$MOTION_TOOLS_DIR` or
// `~/.cache/motionengine/tools/vision_probe` and run per image (results are
// cached by file hash).
//
//   vision_probe <in-image> [--png <out.png>]
//
// Prints one JSON object on stdout:
//   {"width", "height", "has_alpha", "opaque", "humans", "human_area_max",
//    "labels": [top classification labels with confidence >= 0.3]}
// `--png` also writes the image as an sRGB PNG (used for WebP input, which the
// engine's manifests do not accept).
//
// Exit codes: 0 ok, 1 error, 2 usage.
import CoreGraphics
import Foundation
import ImageIO
import UniformTypeIdentifiers
import Vision

func fail(_ msg: String, _ code: Int32 = 1) -> Never {
    FileHandle.standardError.write((msg + "\n").data(using: .utf8)!)
    exit(code)
}

let args = CommandLine.arguments
guard args.count >= 2 else { fail("usage: vision_probe <in-image> [--png <out.png>]", 2) }
let inURL = URL(fileURLWithPath: args[1])
var pngOut: URL? = nil
if let i = args.firstIndex(of: "--png") {
    guard i + 1 < args.count else { fail("--png needs a path", 2) }
    pngOut = URL(fileURLWithPath: args[i + 1])
}

guard let source = CGImageSourceCreateWithURL(inURL as CFURL, nil),
    let image = CGImageSourceCreateImageAtIndex(source, 0, nil)
else { fail("cannot read \(inURL.path)") }

let width = image.width
let height = image.height
let alphaInfo = image.alphaInfo
let hasAlpha = !(alphaInfo == .none || alphaInfo == .noneSkipFirst || alphaInfo == .noneSkipLast)

// Opaque: no alpha channel, or every pixel of a small rendition is (almost) opaque.
func isOpaque() -> Bool {
    if !hasAlpha { return true }
    let scale = min(1.0, 256.0 / Double(max(width, height)))
    let w = max(1, Int(Double(width) * scale))
    let h = max(1, Int(Double(height) * scale))
    guard let space = CGColorSpace(name: CGColorSpace.sRGB),
        let ctx = CGContext(
            data: nil, width: w, height: h, bitsPerComponent: 8, bytesPerRow: w * 4,
            space: space, bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return false }
    ctx.clear(CGRect(x: 0, y: 0, width: w, height: h))
    ctx.draw(image, in: CGRect(x: 0, y: 0, width: w, height: h))
    guard let data = ctx.data else { return false }
    let px = data.bindMemory(to: UInt8.self, capacity: w * h * 4)
    for i in 0..<(w * h) where px[i * 4 + 3] < 250 {
        return false
    }
    return true
}

let opaque = isOpaque()
let handler = VNImageRequestHandler(cgImage: image, options: [:])
let humansRequest = VNDetectHumanRectanglesRequest()
humansRequest.upperBodyOnly = false
let classify = VNClassifyImageRequest()
do {
    try handler.perform([humansRequest, classify])
} catch {
    fail("vision: \(error)")
}
let humans = humansRequest.results ?? []
var areaMax = 0.0
for h in humans {
    areaMax = max(areaMax, Double(h.boundingBox.width * h.boundingBox.height))
}
let labels = (classify.results ?? [])
    .filter { $0.confidence >= 0.3 }
    .sorted { $0.confidence > $1.confidence }
    .prefix(6)
    .map { $0.identifier }

if let out = pngOut {
    guard
        let dest = CGImageDestinationCreateWithURL(
            out as CFURL, UTType.png.identifier as CFString, 1, nil)
    else { fail("cannot write \(out.path)") }
    CGImageDestinationAddImage(dest, image, nil)
    if !CGImageDestinationFinalize(dest) { fail("cannot write \(out.path)") }
}

let result: [String: Any] = [
    "width": width,
    "height": height,
    "has_alpha": hasAlpha,
    "opaque": opaque,
    "humans": humans.count,
    "human_area_max": (areaMax * 10000).rounded() / 10000,
    "labels": Array(labels),
]
guard let json = try? JSONSerialization.data(withJSONObject: result, options: [.sortedKeys]) else {
    fail("cannot encode the result")
}
FileHandle.standardOutput.write(json)
FileHandle.standardOutput.write("\n".data(using: .utf8)!)
