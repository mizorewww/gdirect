// Regenerates gdirect.icns: swift make-icon.swift <MaterialSymbolsRounded.ttf> <dir>
// then iconutil -c icns <dir>/gdirect.iconset -o gdirect.icns

import AppKit
import CoreText

// gdirect's app icon: the filled Material Symbol add_to_drive in white on
// the client's default accent (#415F91), as on its welcome screen.
let args = CommandLine.arguments
let fontURL = URL(fileURLWithPath: args[1])
let out = URL(fileURLWithPath: args[2])
CTFontManagerRegisterFontsForURL(fontURL as CFURL, .process, nil)

func render(_ size: Int) -> Data {
    let s = CGFloat(size)
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    // macOS icon grid: the tile is 824/1024 of the canvas, corner radius 185.
    let inset = s * 100 / 1024
    let tile = NSRect(x: inset, y: inset, width: s - 2 * inset, height: s - 2 * inset)
    let path = NSBezierPath(roundedRect: tile, xRadius: s * 185 / 1024, yRadius: s * 185 / 1024)
    let top = NSColor(srgbRed: 0x55/255, green: 0x75/255, blue: 0xA8/255, alpha: 1)
    let bottom = NSColor(srgbRed: 0x41/255, green: 0x5F/255, blue: 0x91/255, alpha: 1)
    NSGradient(starting: top, ending: bottom)!.draw(in: path, angle: -90)
    // The filled variant of the variable font (FILL axis = 1).
    let base = CTFontCreateWithName("Material Symbols Rounded" as CFString, s * 0.52, nil)
    let fill: [NSNumber: NSNumber] = [NSNumber(value: 0x46494C4C): 1, NSNumber(value: 0x77676874): 500]
    let desc = CTFontDescriptorCreateCopyWithAttributes(CTFontCopyFontDescriptor(base),
        [kCTFontVariationAttribute: fill] as CFDictionary)
    let font = CTFontCreateWithFontDescriptor(desc, s * 0.52, nil)
    let glyph = String(UnicodeScalar(0xe65c)!)
    let text = NSAttributedString(string: glyph, attributes: [
        .font: font, .foregroundColor: NSColor.white])
    let line = CTLineCreateWithAttributedString(text)
    let bounds = CTLineGetBoundsWithOptions(line, .useGlyphPathBounds)
    let ctx = NSGraphicsContext.current!.cgContext
    ctx.textPosition = CGPoint(x: (s - bounds.width) / 2 - bounds.minX,
                               y: (s - bounds.height) / 2 - bounds.minY)
    CTLineDraw(line, ctx)
    NSGraphicsContext.restoreGraphicsState()
    return rep.representation(using: .png, properties: [:])!
}

let iconset = out.appendingPathComponent("gdirect.iconset")
try? FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)
for (name, size) in [("16x16", 16), ("16x16@2x", 32), ("32x32", 32), ("32x32@2x", 64),
                     ("128x128", 128), ("128x128@2x", 256), ("256x256", 256),
                     ("256x256@2x", 512), ("512x512", 512), ("512x512@2x", 1024)] {
    try render(size).write(to: iconset.appendingPathComponent("icon_\(name).png"))
}
