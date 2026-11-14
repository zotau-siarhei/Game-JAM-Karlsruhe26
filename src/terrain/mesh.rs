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
            let a = ChunkData::index(ix, iz) as u32;
            let b = ChunkData::index(ix + 1, iz) as u32;
            let c = ChunkData::index(ix, iz + 1) as u32;
            let d = ChunkData::index(ix + 1, iz + 1) as u32;
            // Обход против часовой стрелки при взгляде сверху: лицевая сторона смотрит вверх.
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }

    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, chunk.colors.clone())
        .with_inserted_indices(Indices::U32(indices))
}
