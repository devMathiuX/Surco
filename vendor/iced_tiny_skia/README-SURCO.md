# iced_tiny_skia (copia de Surco)

Copia de [`iced_tiny_skia` 0.14.1](https://crates.io/crates/iced_tiny_skia),
el renderer por CPU de [iced](https://github.com/iced-rs/iced) (licencia MIT,
ver `LICENSE`). Se usa en lugar del publicado con `[patch.crates-io]` en el
`Cargo.toml` del workspace.

## Cambios

Todos marcados con `Surco:` en el código. Medidos con `perf` desplazando una
lista (detalle y números en docs/DEVELOPMENT.md, "Fluidez del scroll").

- `engine::adjust_clip_mask`: borraba la máscara de recorte entera (del tamaño
  de la ventana) cada vez que cambiaba el recorte. Ahora borra solo el
  rectángulo anterior y escribe el nuevo a mano.
- `text::GlyphCache::allocate`: los glifos vacíos (espacios) no se guardaban y
  se volvían a pedir en cada cuadro, armando de nuevo el hinting de la fuente.
- `engine::fill_clipped`: el relleno de los quads se arma con bandas rectas
  (recortadas a mano, sin máscara) y las cuatro esquinas, en lugar de
  rasterizar el rectángulo redondeado entero por la máscara.
- `window::compositor::present`: con más de 8 rectángulos dañados se dibuja uno
  solo que los cubre (antes lo que tocaba a varios se pintaba varias veces).
- `raster::blit`: imágenes sin rotar, opacas a pleno y sin escalar (o
  escaladas sin suavizar) se copian fila por fila.

Al subir de versión de iced: comparar con el original nuevo, volver a aplicar
los cambios (o borrar la copia si iced ya lo resolvió) y medir con
`SURCO_FRAMES=1` (ver docs/DEVELOPMENT.md).
