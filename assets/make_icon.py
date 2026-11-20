"""Logo de Tube : carré arrondi en dégradé (violet → bleu), écran cathodique stylisé (le « tube »)
au contour blanc, triangle de lecture plein et petit point corail (le voyant « en direct »).
Lisible de 16 à 256 px. Usage : py -3 assets/make_icon.py"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

TOP = (124, 92, 255)      # violet
BOTTOM = (40, 132, 255)   # bleu
WHITE = (255, 255, 255, 255)
CORAL = (255, 112, 96, 255)
S = 1024


def draw(px):
    ss = 4
    W = px * ss
    k = W / S
    s = lambda v: int(round(v * k))
    # fond : dégradé vertical dans un carré arrondi
    grad = Image.new("RGBA", (W, W))
    gd = ImageDraw.Draw(grad)
    for y in range(W):
        t = y / (W - 1)
        c = tuple(int(TOP[i] + (BOTTOM[i] - TOP[i]) * t) for i in range(3)) + (255,)
        gd.line([(0, y), (W, y)], fill=c)
    mask = Image.new("L", (W, W), 0)
    ImageDraw.Draw(mask).rounded_rectangle([s(40), s(40), s(984), s(984)], radius=s(230), fill=255)
    img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    img.paste(grad, (0, 0), mask)
    # ombre douce de l'écran
    sh = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    ImageDraw.Draw(sh).rounded_rectangle([s(190), s(250), s(834), s(790)], radius=s(150), fill=(10, 10, 40, 90))
    sh = sh.filter(ImageFilter.GaussianBlur(s(28)))
    img = Image.alpha_composite(img, sh)
    d = ImageDraw.Draw(img)
    # écran (le « tube ») : contour blanc épais ; plus épais aux petites tailles pour rester lisible
    width = s(64) if px >= 48 else s(90)
    if px > 16:
        d.rounded_rectangle([s(180), s(230), s(844), s(774)], radius=s(140), outline=WHITE, width=width)
    # triangle de lecture, centré optiquement (seul, et plus grand, en 16 px)
    cx, cy, r = (530, 502, 150) if px > 16 else (560, 512, 300)
    d.polygon([(s(cx - r * 0.75), s(cy - r)), (s(cx - r * 0.75), s(cy + r)), (s(cx + r * 0.95), s(cy))], fill=WHITE)
    # voyant corail
    if px >= 24:
        rr = 58
        d.ellipse([s(770 - rr), s(160 - rr), s(770 + rr), s(160 + rr)], fill=CORAL, outline=WHITE, width=s(22))
    return img.resize((px, px), Image.LANCZOS)


here = Path(__file__).parent
draw(1024).save(here / "icon.png")
draw(64).save(here / "icon64.png")
draw(32).save(here / "icon32.png")
frames = [draw(n) for n in (16, 24, 32, 48, 64, 128, 256)]
frames[-1].save(here / "icon.ico", sizes=[(f.width, f.height) for f in frames], append_images=frames[:-1])
print("ok")
