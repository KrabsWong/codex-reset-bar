// Renders the website avatar as a circular macOS icon at every scale.
import AppKit
import Foundation

let output = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "build/AppIcon.iconset"
let logo = NSImage(contentsOfFile: "resources/codex-resets-logo.jpg")!
try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true)

func render(_ pixels: Int, inset: CGFloat) -> Data {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    NSGraphicsContext.current!.imageInterpolation = .high
    let edge = CGFloat(pixels)
    let rect = NSRect(x: edge * inset, y: edge * inset,
        width: edge * (1 - 2 * inset), height: edge * (1 - 2 * inset))
    NSBezierPath(ovalIn: rect).addClip()
    logo.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1)
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

for size in [16, 32, 64, 128, 256, 512, 1024] {
    let name = size == 1024 ? "icon_512x512@2x.png" : "icon_\(size)x\(size).png"
    try render(size, inset: 0.08).write(to: URL(fileURLWithPath: "\(output)/\(name)"))
}
try render(64, inset: 0).write(to: URL(fileURLWithPath: "resources/status-logo.png"))
