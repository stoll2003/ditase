// Gera o ícone do Dita.se: um "D." branco com volume sobre roxo escuro.
// Uso: swift docs/design/icone.swift docs/design/icone-1024.png
//
// Forma e margem são as do ícone clássico do macOS (quadrado arredondado de 824 pt num quadro de
// 1024, cantos de 185): é o que o sistema mostra como está. Arte quadrada de borda a borda, ou
// num catálogo de recursos, é encaixada pelo macOS 26 em diante dentro de uma moldura própria.
import AppKit
import CoreText

let lado: CGFloat = 1024
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: Int(lado), pixelsHigh: Int(lado), bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
let contexto = NSGraphicsContext(bitmapImageRep: rep)!
NSGraphicsContext.current = contexto
let cg = contexto.cgContext

// Fundo: roxo escuro, mais claro em cima, recortado na forma do ícone.
let quadro = NSRect(x: 100, y: 100, width: 824, height: 824)
let recorte = NSBezierPath(roundedRect: quadro, xRadius: 185, yRadius: 185)
cg.saveGState()
recorte.addClip()
NSGradient(colors: [NSColor(red: 0.27, green: 0.21, blue: 0.38, alpha: 1), NSColor(red: 0.09, green: 0.07, blue: 0.13, alpha: 1)])!.draw(in: quadro, angle: -90)
NSGradient(colors: [NSColor(red: 0.62, green: 0.50, blue: 0.90, alpha: 0.22), NSColor(red: 0.62, green: 0.50, blue: 0.90, alpha: 0)])!
    .draw(fromCenter: NSPoint(x: 512, y: 860), radius: 0, toCenter: NSPoint(x: 512, y: 860), radius: 620, options: [])

// Contorno do "D" numa fonte arredondada e pesada.
func contorno(_ texto: String, tamanho: CGFloat) -> CGPath {
    let base = NSFont.systemFont(ofSize: tamanho, weight: .heavy)
    let fonte = base.fontDescriptor.withDesign(.rounded).flatMap { NSFont(descriptor: $0, size: tamanho) } ?? base
    let linha = CTLineCreateWithAttributedString(NSAttributedString(string: texto, attributes: [.font: fonte]))
    let caminho = CGMutablePath()
    for corrida in CTLineGetGlyphRuns(linha) as! [CTRun] {
        let fonteDaCorrida = (CTRunGetAttributes(corrida) as NSDictionary)[kCTFontAttributeName] as! CTFont
        for i in 0..<CTRunGetGlyphCount(corrida) {
            var glifo = CGGlyph(), posicao = CGPoint.zero
            CTRunGetGlyphs(corrida, CFRange(location: i, length: 1), &glifo)
            CTRunGetPositions(corrida, CFRange(location: i, length: 1), &posicao)
            if let forma = CTFontCreatePathForGlyph(fonteDaCorrida, glifo, nil) {
                caminho.addPath(forma, transform: CGAffineTransform(translationX: posicao.x, y: posicao.y))
            }
        }
    }
    return caminho
}

let letra = contorno("D", tamanho: 520)
let caixaDaLetra = letra.boundingBoxOfPath
let raioDoPonto: CGFloat = 47
let vao: CGFloat = 30
let larguraTotal = caixaDaLetra.width + vao + raioDoPonto * 2
let profundidade: CGFloat = 28 // quanto o volume desce
let origemX = (lado - larguraTotal) / 2 - caixaDaLetra.minX
let origemY = (lado - caixaDaLetra.height) / 2 - caixaDaLetra.minY + profundidade / 2
var mover = CGAffineTransform(translationX: origemX, y: origemY)
let forma = CGMutablePath()
forma.addPath(letra, transform: mover)
let centroDoPonto = CGPoint(x: origemX + caixaDaLetra.maxX + vao + raioDoPonto, y: origemY + caixaDaLetra.minY + raioDoPonto)
forma.addEllipse(in: CGRect(x: centroDoPonto.x - raioDoPonto, y: centroDoPonto.y - raioDoPonto, width: raioDoPonto * 2, height: raioDoPonto * 2))

// Sombra projetada no fundo.
cg.saveGState()
cg.setShadow(offset: CGSize(width: 0, height: -26), blur: 46, color: NSColor(white: 0, alpha: 0.55).cgColor)
cg.translateBy(x: 0, y: -profundidade)
cg.addPath(forma)
cg.setFillColor(NSColor(red: 0.20, green: 0.16, blue: 0.28, alpha: 1).cgColor)
cg.fillPath()
cg.restoreGState()

// Lateral do volume: a mesma forma empilhada para baixo, escurecendo.
for passo in stride(from: Int(profundidade), through: 1, by: -1) {
    let t = CGFloat(passo) / profundidade
    cg.saveGState()
    cg.translateBy(x: 0, y: -CGFloat(passo))
    cg.addPath(forma)
    cg.setFillColor(NSColor(red: 0.66 - 0.30 * t, green: 0.62 - 0.30 * t, blue: 0.76 - 0.30 * t, alpha: 1).cgColor)
    cg.fillPath()
    cg.restoreGState()
}

// Face: branco com um leve degradê, para a luz de cima.
cg.saveGState()
cg.addPath(forma)
cg.clip()
let caixa = forma.boundingBoxOfPath
let face = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [NSColor(white: 1, alpha: 1).cgColor, NSColor(red: 0.88, green: 0.85, blue: 0.95, alpha: 1).cgColor] as CFArray, locations: [0, 1])!
cg.drawLinearGradient(face, start: CGPoint(x: 0, y: caixa.maxY), end: CGPoint(x: 0, y: caixa.minY), options: [])
cg.restoreGState()
cg.restoreGState() // fim do recorte do fundo

try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: CommandLine.arguments[1]))
