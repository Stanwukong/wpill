import { useEffect, useState } from "react";

/** Island colors derived from album art. Applied as CSS custom properties (see `@property` in global.css). */
export interface ArtTheme {
  /** Background gradient, top-left → bottom-right. Kept dark so `--island-fg` text stays readable. */
  from: string;
  to: string;
  /** Brighter variant for small highlights (equalizer, playing dot). */
  accent: string;
}

/** The art is downscaled to this many px per side before sampling; plenty for a dominant color. */
const SAMPLE_PX = 24;
const HUE_BUCKETS = 12;
const CACHE_LIMIT = 16;

const cache = new Map<string, ArtTheme | null>();

/** Theme for `artwork` (a data URL). Keeps the previous theme while a new one is computed, so it doesn't flash. */
export function useArtTheme(artwork: string | null): ArtTheme | null {
  const [theme, setTheme] = useState<ArtTheme | null>(() => (artwork ? (cache.get(artwork) ?? null) : null));

  useEffect(() => {
    if (!artwork) {
      setTheme(null);
      return;
    }
    const cached = cache.get(artwork);
    if (cached !== undefined) {
      setTheme(cached);
      return;
    }
    let alive = true;
    extractTheme(artwork)
      .catch(() => null)
      .then((t) => {
        if (cache.size >= CACHE_LIMIT) cache.delete(cache.keys().next().value!);
        cache.set(artwork, t);
        if (alive) setTheme(t);
      });
    return () => {
      alive = false;
    };
  }, [artwork]);

  return theme;
}

async function extractTheme(dataUrl: string): Promise<ArtTheme | null> {
  const img = new Image();
  img.src = dataUrl;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = canvas.height = SAMPLE_PX;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.drawImage(img, 0, 0, SAMPLE_PX, SAMPLE_PX);
  return themeFromPixels(ctx.getImageData(0, 0, SAMPLE_PX, SAMPLE_PX).data);
}

/**
 * Picks the dominant *colorful* hue: pixels are bucketed by hue and weighted by saturation and
 * closeness to mid-lightness, so a small vivid area beats a large grey or black one.
 * Mostly-grey art falls back to its average color, which gives a neutral tint.
 */
export function themeFromPixels(rgba: Uint8ClampedArray): ArtTheme | null {
  const buckets = Array.from({ length: HUE_BUCKETS }, () => ({ w: 0, r: 0, g: 0, b: 0 }));
  const all = { n: 0, r: 0, g: 0, b: 0 };

  for (let i = 0; i < rgba.length; i += 4) {
    if (rgba[i + 3] < 128) continue;
    const r = rgba[i], g = rgba[i + 1], b = rgba[i + 2];
    all.n += 1;
    all.r += r;
    all.g += g;
    all.b += b;
    const [h, s, l] = rgbToHsl(r, g, b);
    if (s < 0.2 || l < 0.1 || l > 0.9) continue;
    const w = s * (1 - Math.abs(l - 0.5) * 1.4);
    const bucket = buckets[Math.floor(h * HUE_BUCKETS) % HUE_BUCKETS];
    bucket.w += w;
    bucket.r += r * w;
    bucket.g += g * w;
    bucket.b += b * w;
  }
  if (all.n === 0) return null;

  const best = buckets.reduce((a, b) => (b.w > a.w ? b : a));
  // Less than ~3% of the image is meaningfully colorful: treat it as greyscale art.
  const [h, s] =
    best.w > all.n * 0.03
      ? rgbToHsl(best.r / best.w, best.g / best.w, best.b / best.w)
      : rgbToHsl(all.r / all.n, all.g / all.n, all.b / all.n);

  const bgSat = Math.min(s, 0.6);
  return {
    from: hsl(h, bgSat, 0.24),
    to: hsl(h, bgSat * 0.8, 0.09),
    accent: hsl(h, Math.min(Math.max(s, 0.45), 0.9), 0.66),
  };
}

function rgbToHsl(r: number, g: number, b: number): [number, number, number] {
  r /= 255;
  g /= 255;
  b /= 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const l = (max + min) / 2;
  const d = max - min;
  if (d === 0) return [0, 0, l];
  const s = d / (1 - Math.abs(2 * l - 1));
  let h: number;
  if (max === r) h = ((g - b) / d) % 6;
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [((h / 6) + 1) % 1, s, l];
}

function hsl(h: number, s: number, l: number): string {
  return `hsl(${Math.round(h * 360)} ${Math.round(s * 100)}% ${Math.round(l * 100)}%)`;
}
