# Optional Local Fonts

The application currently uses local Windows/system font fallbacks, so no WOFF2 files are required
for the landing page or desktop build. Keep any future font files in this directory and add matching
`@font-face` declarations only after verifying that the files are committed and resolve in both Vite
entrypoints.
