# Animated pictures and animated stickers (issue #96)

**Written 2026-10-19** against `v1.3`, in answer to issue #96: *"We should add support for animated webpages
and gif files."* — and, from the owner, *"support animation and especially support animation in stickers."*
The wire is in `docs/protocol.md`, "Animations" (new) and "Sticker pack" (amended); this is the reasoning.

## The owner's choices (2026-10-19)

1. **A GIF or animated WebP sent in a chat becomes a silent looping video** — Telegram's way — rather than
   travelling as the picture (offered as the recommendation) or staying a file.
2. **Stickers stay WebP (and PNG).** An animated GIF is still refused as a sticker.
3. **Animations play by themselves and loop**, and show their first frame when the system asks for less
   motion.
4. **Stickers animate in the sticker panel and the family's pack too**, on every client.

## What each client did before (audited 2026-10-19)

| | Web | Android | Apple | Windows |
|---|---|---|---|---|
| Animated WebP sticker, in the chat | plays (`<img>`) | plays on API 28+ (`AnimatedImageDrawable`) | plays (`CGAnimateImageDataWithBlock`) | plays (WIC frames on a timer) — never run on Windows |
| Sticker panel, pack grid | play | play | still, on purpose | still |
| Enlarged sticker | plays | plays | plays | plays |
| GIF / animated WebP in a chat | a file row | a file row — or, from the PHOTO PICKER, a still JPEG (a bug) | a file row | a file row |
| Animated PNG in the composer, on the board | flattened to a still JPEG, unsaid | the same | the same | the same |
| Reduce Motion for stickers | not honoured | not honoured | not honoured | honoured |

So stickers mostly worked. What did not was everything else: an animation sent in a chat never moved anywhere,
and three different ways of losing one (the Android picker, APNG everywhere, the board) did it without a word.

## Why a video

- **One moving format every client plays the same way.** H.264 in MP4 is already the profile; every client
  has a player for it and an encoder for it (the #74 transcoders, the #79 recorders).
- **Size.** A GIF stores every frame as a palette image with LZW; the same frames as H.264 are commonly a
  tenth of the size or less. That is what makes the next point affordable.
- **It plays by itself.** A tile has to download what it plays, which the protocol otherwise forbids for
  videos; an animation is allowed to, up to 8 MiB.
- **The price is transparency**: a GIF's empty corners are drawn on white. That is why stickers are left out
  — a sticker's point is that the chat shows through it.

## The timing, exactly

Four encoders must make the same clip, so the timing is a shared function with vectors
(`fc_text::animation`, `animation-plan-vectors.json`, copied beside the C#, Swift and Kotlin tests and
compared by CI). Sampling on a 30-a-second grid — rather than keeping or dropping source frames — is what
keeps a frame held for a second after a 20 ms flash held, keeps the profile's 30-frame ceiling, and never
gives a slow animation frames it did not have. The shortest-frame rule (≤ 10 ms → 100 ms) is the browsers'
own, so a GIF that says "0" moves here as it moves on a web page.

## Where it is built

| Part | Where |
|---|---|
| The rule | `web/text/src/animation.rs`, vectors from `win/tools/board-oracle` (`animation-plan`) |
| The flag | the server: a migration (`animation` on the attachment, three CHECKs: only on a video, never beside `sticker` or `round`), the send checks, every read, the push word |
| Web | WebCodecs `ImageDecoder` (composited frames, durations) → `VideoEncoder` → the existing MP4 writer |
| Apple | ImageIO (`CGImageSource`, composited frames and delays) → `AVAssetWriter` |
| Android | each format's frames composited by the client → `MediaCodec` / `MediaMuxer` (or Media3) |
| Windows | WIC frames composited by the client → Media Foundation |

## Not verified here

Real phone keyboards and pickers aside, the parts only a device can show: what each platform's photo picker
hands over for an animated GIF (some convert), WIC's animated-WebP frames on Windows, Android hardware
encoders' handling of very small sizes, and how many animations can play at once before a phone stutters.
