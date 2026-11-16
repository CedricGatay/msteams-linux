#!/usr/bin/env swift
// Original vector artwork. Run from any directory; only macOS AppKit is needed.
// This is an asset-maintenance tool, not a dependency of end-user exports.
import AppKit

let root = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent()
let output = root.appendingPathComponent("src-tauri/icons/generator", isDirectory: true)
let files = FileManager.default
try files.createDirectory(at: output, withIntermediateDirectories: true)

func color(_ hex: UInt32, _ alpha: CGFloat = 1) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 255) / 255,
            green: CGFloat((hex >> 8) & 255) / 255,
            blue: CGFloat(hex & 255) / 255, alpha: alpha)
}

func render(_ pixels: Int) throws -> Data {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    let context = NSGraphicsContext(bitmapImageRep: bitmap)!.cgContext
    context.scaleBy(x: CGFloat(pixels) / 1024, y: CGFloat(pixels) / 1024)
    context.translateBy(x: 0, y: 1024)
    context.scaleBy(x: 1, y: -1)
    context.setAllowsAntialiasing(true)

    func rounded(_ rect: CGRect, _ radius: CGFloat) -> CGPath {
        CGPath(roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)
    }
    func fill(_ path: CGPath, _ value: CGColor) {
        context.addPath(path); context.setFillColor(value); context.fillPath()
    }
    func gradient(_ path: CGPath, _ top: UInt32, _ bottom: UInt32, _ start: CGPoint, _ end: CGPoint) {
        context.saveGState(); context.addPath(path); context.clip()
        let gradient = CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB),
            colors: [color(top), color(bottom)] as CFArray, locations: [0, 1])!
        context.drawLinearGradient(gradient, start: start, end: end,
            options: [.drawsBeforeStartLocation, .drawsAfterEndLocation])
        context.restoreGState()
    }
    func polygon(_ points: [CGPoint]) -> CGPath {
        let path = CGMutablePath(); path.addLines(between: points); path.closeSubpath(); return path
    }
    func tile(_ x: CGFloat, _ y: CGFloat, _ size: CGFloat, _ top: UInt32, _ bottom: UInt32) {
        context.saveGState()
        context.setShadow(offset: CGSize(width: 0, height: 12), blur: 20, color: color(0x040D2A, 0.3))
        gradient(rounded(CGRect(x: x, y: y, width: size, height: size), size * 0.22),
            top, bottom, CGPoint(x: x, y: y), CGPoint(x: x + size, y: y + size))
        context.restoreGState()
        fill(rounded(CGRect(x: x + size * 0.21, y: y + size * 0.25,
            width: size * 0.58, height: size * 0.11), size * 0.04), color(0xFFFFFF, 0.9))
        fill(rounded(CGRect(x: x + size * 0.21, y: y + size * 0.47,
            width: size * 0.4, height: size * 0.08), size * 0.04), color(0xFFFFFF, 0.55))
    }

    let background = rounded(CGRect(x: 64, y: 64, width: 896, height: 896), 196)
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: 15), blur: 25, color: color(0x06102F, 0.25))
    gradient(background, 0x174D86, 0x342774, CGPoint(x: 180, y: 90), CGPoint(x: 850, y: 960))
    context.restoreGState()

    // Unified cloud silhouette: the source of the generated applications.
    let cloud = CGMutablePath()
    cloud.move(to: CGPoint(x: 305, y: 530))
    cloud.addCurve(to: CGPoint(x: 285, y: 326), control1: CGPoint(x: 169, y: 530), control2: CGPoint(x: 163, y: 342))
    cloud.addCurve(to: CGPoint(x: 544, y: 280), control1: CGPoint(x: 305, y: 173), control2: CGPoint(x: 484, y: 159))
    cloud.addCurve(to: CGPoint(x: 744, y: 354), control1: CGPoint(x: 625, y: 236), control2: CGPoint(x: 725, y: 268))
    cloud.addCurve(to: CGPoint(x: 731, y: 530), control1: CGPoint(x: 861, y: 363), control2: CGPoint(x: 863, y: 530))
    cloud.closeSubpath()
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: 10), blur: 22, color: color(0x06102F, 0.2))
    gradient(cloud, 0xFFFFFF, 0xD6EEFF, CGPoint(x: 400, y: 220), CGPoint(x: 550, y: 530))
    context.restoreGState()

    // A downward creation/export arrow connects cloud services to app bundles.
    fill(polygon([CGPoint(x: 486, y: 373), CGPoint(x: 538, y: 373),
        CGPoint(x: 538, y: 444), CGPoint(x: 577, y: 444), CGPoint(x: 512, y: 506),
        CGPoint(x: 447, y: 444), CGPoint(x: 486, y: 444)]), color(0x2761A1))

    tile(185, 592, 178, 0x2ED4B6, 0x087E92)
    tile(661, 592, 178, 0xFFBD68, 0xE66A4A)
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: 16), blur: 25, color: color(0x030C2A, 0.35))
    gradient(rounded(CGRect(x: 384, y: 563, width: 256, height: 256), 55),
        0xFFFFFF, 0xD9E7FF, CGPoint(x: 420, y: 563), CGPoint(x: 610, y: 820))
    context.restoreGState()
    for (x, y, value): (CGFloat, CGFloat, UInt32) in [
        (438, 619, 0x6570DA), (522, 619, 0x168BD3),
        (438, 703, 0x20AA8F), (522, 703, 0xF18D4C)
    ] {
        fill(rounded(CGRect(x: x, y: y, width: 64, height: 64), 15), color(value))
    }

    // Four-point sparkle: creation, distinct from any service's own logo.
    fill(polygon([CGPoint(x: 789, y: 185), CGPoint(x: 811, y: 245),
        CGPoint(x: 871, y: 267), CGPoint(x: 811, y: 289), CGPoint(x: 789, y: 349),
        CGPoint(x: 767, y: 289), CGPoint(x: 707, y: 267), CGPoint(x: 767, y: 245)]), color(0xFFE5A0))
    return bitmap.representation(using: .png, properties: [:])!
}

try render(1024).write(to: output.appendingPathComponent("icon.png"))
try render(128).write(to: root.appendingPathComponent("dist/generator.png"))
let scratch = files.temporaryDirectory.appendingPathComponent("cloud-generator-\(UUID().uuidString)")
let iconset = scratch.appendingPathComponent("Generator.iconset", isDirectory: true)
try files.createDirectory(at: iconset, withIntermediateDirectories: true)
defer { try? files.removeItem(at: scratch) }
for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let suffix = scale == 2 ? "@2x" : ""
        try render(points * scale).write(to: iconset.appendingPathComponent("icon_\(points)x\(points)\(suffix).png"))
    }
}
let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconset.path, "-o", output.appendingPathComponent("icon.icns").path]
try process.run(); process.waitUntilExit()
guard process.terminationStatus == 0 else { fatalError("iconutil failed") }
print("Generator icon assets written to \(output.path) and dist/generator.png")
