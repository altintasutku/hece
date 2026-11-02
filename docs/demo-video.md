# Demo video

The launch demo ([#58](https://github.com/altintasutku/hece/issues/58)) is a 20–30 s clip with
burned-in captions. The script and shot list are in the issue comment. This page lists where the
video goes and the exact edits to make once it exists.

## Export specs (from #58)

| Output | Spec | Used in |
| --- | --- | --- |
| Master | 1920x1080, 60 fps, H.264 high profile, AAC 48 kHz stereo, CRF ~18 | archive, YouTube, Product Hunt |
| README / site MP4 | 1920x1080 or 1280x720, 30 fps, H.264 + AAC, faststart (`-movflags +faststart`), **< 10 MB** | README, website, Show HN link target |
| Social MP4 | 1280x720 or 1920x1080, 30 fps, H.264 + AAC, captions burned in | X, LinkedIn, Reddit |
| GIF | 960 px wide, 12–15 fps, shots 1–3 only (about 10 s), ffmpeg `palettegen`/`paletteuse`, **< 5 MB** | places that do not play video |
| Poster frame | shot 2 right after the paste | README and site thumbnail, Product Hunt gallery |

Captions: the site font or Inter, white on a 60% dark band, at least 40 px tall at 1080p, one
line, no emoji, no exclamation marks. The 10 MB limit for GitHub attachments on free plans is
not verified; stay under it anyway.

## Where it goes

### README

`README.md` has a marker under the badges:

```md
<!-- DEMO: replace with the demo video (#58) -->
[![Hece: hold a hotkey, speak, and the text lands where you type](site/og.png)](https://altintasutku.github.io/hece/)
```

GitHub only plays videos that are uploaded as attachments (a repo-relative `.mp4` link is not
embedded). To replace the placeholder:

1. Open any issue or PR comment box on GitHub, drag the README MP4 in, and copy the
   `https://github.com/user-attachments/assets/<id>` URL it produces. Do not submit the comment.
2. Replace the marker and the image line with that URL on its own line (GitHub renders a bare
   attachment URL as a player):

   ```md
   https://github.com/user-attachments/assets/<id>
   ```

### Website

`site/index.html` has a hidden `<figure class="demo">` below the hero; the styles are already in
`site/styles.css`.

1. Save the README MP4 as `site/demo.mp4` and the poster as `site/demo-poster.jpg` (1280x720,
   JPEG, ideally under 200 KB).
2. In `site/index.html`, remove `hidden` from `<figure class="demo" hidden>` and uncomment the
   `<video>` inside it (it is `muted autoplay loop playsinline` with `preload="metadata"`).
3. Check the page at desktop and mobile widths before merging; the Pages workflow publishes on
   merge to `main`.

### Release notes

Add the attachment URL from the README step near the top of `docs/releases/<tag>.md`; the release
workflow uses that file as the release body.

### Launch posts

Show HN links to the repo (the README plays the video). X, LinkedIn and Reddit get the social MP4
uploaded natively; use the GIF only where video does not play.
