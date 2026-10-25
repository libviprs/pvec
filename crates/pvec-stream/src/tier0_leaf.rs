//! Tier 0 per index leaf, as container block kind 8 (issue P5.4b, pvec#149).
//!
//! The writer cuts the whole-artifact [`Tier0Plan`] by the leaf each entity sits in and stores one block
//! per leaf, in the leaf's cell, so a reader that has the spatial directory fetches the first paint of a
//! viewport as the blocks of the leaves it touches and not as one global stream. The layout is the plan's
//! stream at version 2 (`docs/STREAMING.md` section 4.4); the rules for the block are TIER0-10 to
//! TIER0-14 (section 4.5).

use crate::directory::{Aabb, Cell, CellId, EntityBox, SpatialDirectory};
use crate::tiers::{Tier0Error, Tier0Plan};
use pvec_container::directory::{BlockEntry, BlockKind, STRADDLER_CELL_FIRST};
use pvec_container::source::BlockSource;
use pvec_container::{BlockId, Bounds, ContainerReader};
use pvec_entity::EntityId;
use std::collections::BTreeMap;

/// The Tier 0 block of one leaf, ready for `ContainerWriter::push_tier0_leaf`.
#[derive(Clone, Debug, PartialEq)]
pub struct Tier0Block {
    /// The leaf, which is the block's cell.
    pub cell: CellId,
    /// The entities the block covers, which is the leaf's entity count.
    pub count: u32,
    /// The leaf's bounds: the union of its entities' boxes.
    pub bounds: Aabb,
    /// The block.
    pub bytes: Vec<u8>,
}

/// Cuts `plan` by leaf. `entities` and `cells` are what the spatial directory was built from and what it
/// answered (`BuiltDirectory::cells`), in the same order. One block per leaf, ascending by cell. An
/// entity the plan has no entry for is `Gap`.
pub fn tier0_blocks(
    plan: &Tier0Plan,
    entities: &[EntityBox],
    cells: &[CellId],
) -> Result<Vec<Tier0Block>, Tier0Error> {
    if entities.len() != cells.len() {
        return Err(Tier0Error::Format { reason: "an entity has no cell" });
    }
    let mut leaves: BTreeMap<CellId, (Vec<EntityId>, Aabb)> = BTreeMap::new();
    for (e, cell) in entities.iter().zip(cells) {
        let leaf = leaves.entry(*cell).or_insert_with(|| (Vec::new(), e.bounds));
        leaf.0.push(e.id);
        leaf.1 = leaf.1.union(&e.bounds);
    }
    leaves
        .into_iter()
        .map(|(cell, (ids, bounds))| {
            let bytes = plan.cut(&ids)?.encode();
            Ok(Tier0Block { cell, count: ids.len() as u32, bounds, bytes })
        })
        .collect()
}

/// The Tier 0 blocks of an open file, by leaf.
#[derive(Clone, Debug)]
pub struct Tier0Leaves {
    leaves: BTreeMap<u32, (BlockId, Cell)>,
}

impl Tier0Leaves {
    /// Checks the Tier 0 entries among `entries` against the directory: a cell that is 0 or a straddler
    /// payload cell is TIER0-11, one that is not a leaf TIER0-10, two blocks for a leaf TIER0-12, and a
    /// count or bounds that are not the leaf's TIER0-13. A file with none gives an empty set: the block
    /// is optional.
    pub fn new(entries: &[BlockEntry], dir: &SpatialDirectory) -> Result<Tier0Leaves, Tier0Error> {
        let cells: BTreeMap<u32, Cell> = dir.query(&dir.bounds()).map(|c| (c.id.get(), c)).collect();
        let mut leaves = BTreeMap::new();
        for (i, e) in entries.iter().enumerate().filter(|(_, e)| e.block_kind() == Some(BlockKind::Tier0)) {
            if e.cell == 0 || e.cell >= STRADDLER_CELL_FIRST {
                return Err(Tier0Error::BadCell(e.cell));
            }
            let Some(cell) = cells.get(&e.cell) else { return Err(Tier0Error::NoLeaf(e.cell)) };
            if leaves.contains_key(&e.cell) {
                return Err(Tier0Error::DuplicateLeaf(e.cell));
            }
            if e.count != cell.entities || e.bounds != Some(Bounds::from(cell.bounds)) {
                return Err(Tier0Error::LeafMismatch(e.cell));
            }
            leaves.insert(e.cell, (BlockId(i as u32), *cell));
        }
        Ok(Tier0Leaves { leaves })
    }

    /// The number of leaves that have a block.
    pub fn len(&self) -> usize {
        self.leaves.len()
    }

    /// Whether no leaf has one.
    pub fn is_empty(&self) -> bool {
        self.leaves.is_empty()
    }

    fn read<S: BlockSource>(&self, reader: &ContainerReader<S>, leaf: CellId) -> Result<Option<(Tier0Plan, Cell)>, Tier0Error> {
        let Some((id, cell)) = self.leaves.get(&leaf.get()) else { return Ok(None) };
        let block = reader.read_block(*id).map_err(|e| Tier0Error::Read(e.to_string()))?;
        Ok(Some((Tier0Plan::decode_leaf(block.as_bytes())?, *cell)))
    }

    /// The Tier 0 plan of `leaf`: one range read, the block as stored. `None` when the leaf has no block.
    /// The stream is parsed and held to the leaf's entity count (TIER0-13); that it names the leaf's own
    /// entities is [`Tier0Leaves::verify`], which reads the leaf's hot blocks.
    pub fn tier0<S: BlockSource>(
        &self,
        reader: &ContainerReader<S>,
        leaf: CellId,
    ) -> Result<Option<Tier0Plan>, Tier0Error> {
        let Some((plan, cell)) = self.read(reader, leaf)? else { return Ok(None) };
        if plan.covered() != cell.entities as usize {
            return Err(Tier0Error::LeafMismatch(leaf.get()));
        }
        Ok(Some(plan))
    }

    /// [`Tier0Leaves::tier0`] and the full coverage check: the plan must cover exactly the entities of the
    /// leaf's hot blocks (`Gap`, TIER0-01, or `OutOfRange`, TIER0-03, naming the first that disagrees).
    pub fn verify<S: BlockSource>(
        &self,
        reader: &ContainerReader<S>,
        leaf: CellId,
    ) -> Result<Option<Tier0Plan>, Tier0Error> {
        let Some(plan) = self.tier0(reader, leaf)? else { return Ok(None) };
        let Some((_, cell)) = self.leaves.get(&leaf.get()) else { return Ok(None) };
        let (first, count) = (cell.blocks.first.0, cell.blocks.count);
        let mut ids = Vec::with_capacity(cell.entities as usize);
        for b in first..first + count {
            let block = reader.read_block(BlockId(b)).map_err(|e| Tier0Error::Read(e.to_string()))?;
            let view = block.hot().map_err(|e| Tier0Error::Read(e.to_string()))?;
            for raw in view.entity_ids() {
                ids.push(EntityId::new(*raw).map_err(|_| Tier0Error::Limit)?);
            }
        }
        plan.check_covers(&ids)?;
        Ok(Some(plan))
    }
}
