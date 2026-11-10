# PVec features

This file is generated from `docs/features.toml`. I do not edit it by hand: the gate regenerates it and fails if the committed file differs by a single byte. To change it I change the toml and run `PVEC_BLESS_FEATURES=1 cargo test -p pvec-checks --test issue_299_features`.

A capability says a stream exists. A feature says which part of it is written (producer), worked out (driver, the `SUPPORTED` list of `pvec-scene`) and drawn (consumer, the `CONSUMED` list). Every other row has a golden file that exercises it. A `none.` row is something the format refuses on purpose.

| feature | what | producer | driver | consumer | golden |
|---|---|---|---|---|---|
| `23.stroke_item` | A run of strokes in the paint order. | yes | yes | yes | `tests/compat/blueprint-mix-23-paint-order.pvec` |
| `23.fill_item` | A run of hatch fills in the paint order, so a stroke can sit under a fill. | yes | yes | yes | `tests/compat/blueprint-mix-23-paint-order.pvec` |
| `23.raster_item` | A raster named in the paint order, so a picture is painted where the PDF paints it. | yes | yes | no | `tests/compat/blueprint-mix-23-paint-order.pvec` |
| `23.text_item` | A text record named in the paint order. | yes | yes | no | `tests/compat/blueprint-mix-23-paint-order.pvec` |
| `24.colour` | A stroke colour per entity. | yes | no | yes | `tests/compat/blueprint-mix-24-stroke-paint.pvec` |
| `24.alpha` | Stroke alpha in the low byte of the colour word. | yes | no | yes | `tests/compat/blueprint-mix-24-stroke-paint.pvec` |
| `25.rule.nonzero` | Nonzero winding over all the rings of a hatch record. | yes | no | yes | `tests/compat/blueprint-mix-25-fill-paint.pvec` |
| `25.rule.evenodd` | Even-odd over all the rings of a hatch record. | yes | no | yes | `tests/compat/blueprint-mix-25-fill-paint.pvec` |
| `25.gradient.axial` | Axial gradient as sampled stops. | yes | yes | yes | `tests/compat/blueprint-mix-25-fill-paint.pvec` |
| `25.gradient.radial` | Radial gradient, the PDF type 3 circle family. | yes | yes | yes | `tests/compat/blueprint-mix-25-radial-tiling.pvec` |
| `25.gradient.extend` | Paint before the first stop or after the last. | yes | yes | yes | `tests/compat/blueprint-mix-25-fill-paint.pvec` |
| `25.tiling.coloured` | Tiling pattern with its own colours. | yes | yes | yes | `tests/compat/blueprint-mix-25-radial-tiling.pvec` |
| `25.tiling.uncoloured` | Tiling pattern painted in the colour of the hatch record. | yes | yes | yes | `tests/compat/blueprint-mix-25-fill-paint.pvec` |
| `26.clip` | A clip region of entity rings with a rule, applied to named items. | yes | yes | yes | `tests/compat/blueprint-mix-26-clip.pvec` |
| `26.nested` | A clip intersected with its parent. | yes | yes | yes | `tests/compat/blueprint-mix-26-clip.pvec` |
| `none.blend_mode` | Blend modes other than normal. Refused: alpha is straight and there is no blend operator in the format. | no | no | no | none |
| `none.shading.mesh` | Mesh shadings (PDF types 4 to 7). Refused: a mesh is not a gradient and the format holds no PDF function. | no | no | no | none |
| `none.soft_mask` | A soft mask as a paint property. A raster carries its mask as alpha instead. Refused: a mask would need an offscreen pass the format does not describe. | no | no | no | none |
