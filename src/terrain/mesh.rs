//! Меш участка из данных хранилища.

use bevy::{
    asset::RenderAssetUsages,
    mesh::{Indices, PrimitiveTopology},
    prelude::*,
};

use super::storage::{CHUNK_VERTS, ChunkData, Terrain, chunk_origin};

/// Строит меш участка в локальных координатах: угол участка в начале координат.
///
/// Участок должен быть уже сгенерирован (`Terrain::chunk`). Нормали гладкие, на краях они
/// считаются по высотам соседних участков, чтобы на стыках не было швов освещения.
pub fn build_chunk_mesh(terrain: &Terrain, coord: IVec2) -> Mesh {
    let chunk = terrain
        .get_chunk(coord)
        .expect("меш строится только для сгенерированного участка");
    let origin = chunk_origin(coord).as_ivec2();
    let n = CHUNK_VERTS as i32;
    // Высота вершины с локальными координатами, в том числе за краем участка.
    let height = |ix: i32, iz: i32| -> f32 {
        if (0..n).contains(&ix) && (0..n).contains(&iz) {
            chunk.height(ix as usize, iz as usize)
        } else {
            terrain.vertex_height(origin.x + ix, origin.y + iz)
        }
    };

    let mut positions = Vec::with_capacity(CHUNK_VERTS * CHUNK_VERTS);
    let mut normals = Vec::with_capacity(CHUNK_VERTS * CHUNK_VERTS);
    for iz in 0..n {
        for ix in 0..n {
            positions.push([ix as f32, height(ix, iz), iz as f32]);
            let dx = height(ix + 1, iz) - height(ix - 1, iz);
            let dz = height(ix, iz + 1) - height(ix, iz - 1);
            normals.push(Vec3::new(-dx, 2.0, -dz).normalize().to_array());
        }
    }

    let cells = CHUNK_VERTS - 1;
    let mut indices = Vec::with_capacity(cells * cells * 6);
    for iz in 0..cells {
        for ix in 0..cells {
            let corners = [(ix, iz), (ix + 1, iz), (ix, iz + 1), (ix + 1, iz + 1)];
            let [a, b, c, d] = corners.map(|(x, z)| ChunkData::index(x, z) as u32);
            let [ha, hb, hc, hd] = corners.map(|(x, z)| chunk.height(x, z));
            indices.extend_from_slice(&quad_triangles([a, b, c, d], [ha, hb, hc, hd]));
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, chunk.colors.clone())
        .with_inserted_indices(Indices::U32(indices))
}

/// Два треугольника клетки с углами `a (x, z)`, `b (x+1, z)`, `c (x, z+1)`, `d (x+1, z+1)`.
///
/// Диагональ соединяет пару углов с более близкой высотой: она идёт вдоль линии уровня, и гребни
/// и края кратеров не режутся поперёк в зубья, как при одной диагонали на всю сетку. Обход
/// против часовой стрелки при взгляде сверху: лицевая сторона смотрит вверх.
fn quad_triangles([a, b, c, d]: [u32; 4], [ha, hb, hc, hd]: [f32; 4]) -> [u32; 6] {
    if (hb - hc).abs() <= (ha - hd).abs() {
        [a, c, b, b, c, d]
    } else {
        [a, c, d, a, d, b]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Нормаль треугольника по углам клетки 0..4 в координатах (x, z) и высотам.
    fn normal(tri: &[u32], heights: [f32; 4]) -> Vec3 {
        let p = |i: u32| Vec3::new((i % 2) as f32, heights[i as usize], (i / 2) as f32);
        let [p0, p1, p2] = [p(tri[0]), p(tri[1]), p(tri[2])];
        (p1 - p0).cross(p2 - p0)
    }

    #[test]
    fn diagonal_follows_the_contour_and_faces_up() {
        // Гребень вдоль диагонали a–d: высоты a и d близки, b и c по разные стороны склона.
        // Ребром должна стать a–d, иначе гребень разрезан поперёк.
        let ridge = [1.0, 0.0, 0.3, 0.9];
        let tris = quad_triangles([0, 1, 2, 3], ridge);
        assert!(tris[..3].contains(&0) && tris[..3].contains(&3), "{tris:?}");
        // Гребень вдоль b–c: прежняя диагональ.
        assert_eq!(quad_triangles([0, 1, 2, 3], [0.0, 1.0, 0.9, 0.3]), [0, 2, 1, 1, 2, 3]);
        // При любой диагонали оба треугольника смотрят вверх.
        for heights in [ridge, [0.0, 1.0, 1.0, 0.0], [0.0; 4]] {
            let tris = quad_triangles([0, 1, 2, 3], heights);
            for tri in tris.chunks(3) {
                assert!(normal(tri, heights).y > 0.0, "{tri:?} при {heights:?}");
            }
        }
    }
}
