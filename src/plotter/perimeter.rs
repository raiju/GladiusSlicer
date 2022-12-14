use gladius_shared::settings::LayerSettings;
use gladius_shared::types::{Move, MoveChain, MoveType};

use geo::prelude::*;
use geo::*;
use geo::coords_iter::CoordsIter;
use geo::Geometry::Line;

use crate::PolygonOperations;
use itertools::Itertools;

pub fn inset_polygon_recursive(
    poly: &MultiPolygon<f64>,
    settings: &LayerSettings,
    outer_perimeter: bool,
    layer_left: usize,
) -> Option<MoveChain> {
    let mut move_chains = vec![];
    let inset_poly = poly.offset_from(-settings.layer_width / 2.0);

    for raw_polygon in inset_poly.0.iter() {
        let polygon = raw_polygon.simplify(&0.01);
        let mut outer_chains = vec![];
        let mut inner_chains = vec![];
        let moves = polygon
            .exterior()
            .0
            .iter()
            .circular_tuple_windows::<(_, _)>()
            .map(|(&_start, &end)| Move {
                end,
                move_type: if outer_perimeter {
                    MoveType::OuterPerimeter
                } else {
                    MoveType::InnerPerimeter
                },
                width: settings.layer_width,
            })
            .collect();

        outer_chains.push(MoveChain {
            start_point: polygon.exterior()[0],
            moves,
        });

        for interior in polygon.interiors() {
            let mut moves = vec![];
            for (&_start, &end) in interior.0.iter().circular_tuple_windows::<(_, _)>() {
                moves.push(Move {
                    end,
                    move_type: if outer_perimeter {
                        MoveType::OuterPerimeter
                    } else {
                        MoveType::InnerPerimeter
                    },
                    width: settings.layer_width,
                });
            }
            outer_chains.push(MoveChain {
                start_point: interior.0[0],
                moves,
            });
        }

        if layer_left != 0 {
            let rec_inset_poly = polygon.offset_from(-settings.layer_width / 2.0);

            for polygon_rec in rec_inset_poly {
                if let Some(mc) = inset_polygon_recursive(
                    &MultiPolygon::from(polygon_rec),
                    settings,
                    false,
                    layer_left - 1,
                ) {
                    inner_chains.push(mc);
                }
            }
        }

        if settings.inner_perimeters_first {
            move_chains.append(&mut inner_chains);
            move_chains.append(&mut outer_chains);
        } else {
            move_chains.append(&mut outer_chains);
            move_chains.append(&mut inner_chains);
        }
    }

    collapse_move_chains(move_chains)
}

pub fn draw_as_line(
    poly: &Polygon<f64>,
    settings: &LayerSettings,
    move_type: MoveType,
) -> Option<MoveChain> {

    // TODO: Draw line(s) of one layer-width that approximates polygon

    // Naive simple approach/cheat to prove out overhang.stl before going to MAT/SAT
    let boundary = poly.bounding_rect()?;

    if boundary.width() > boundary.height() {
        Some(MoveChain {
            start_point: Coordinate { x: boundary.min().x, y: boundary.min().y + boundary.height() / 2.0 },
            moves: vec![
                Move {
                    end: Coordinate { x: boundary.max().x, y: boundary.max().y - boundary.height() / 2.0 },
                    width: settings.layer_width,
                    move_type,
                }
            ],
        })
    } else {
        Some(MoveChain {
            start_point: Coordinate { x: boundary.min().x + boundary.width() / 2.0, y: boundary.min().y },
            moves: vec![
                Move {
                    end: Coordinate { x: boundary.max().x - boundary.width() / 2.0, y: boundary.max().y },
                    width: settings.layer_width,
                    move_type,
                }
            ],
        })
    }
}

fn collapse_move_chains(move_chains: Vec<MoveChain>) -> Option<MoveChain> {
    move_chains
        .get(0)
        .map(|mc| mc.start_point)
        .map(|starting_point| {
            let mut full_moves = vec![];

            for mut chain in move_chains {
                full_moves.push(Move {
                    end: chain.start_point,
                    move_type: MoveType::Travel,
                    width: 0.0,
                });
                full_moves.append(&mut chain.moves)
            }

            MoveChain {
                moves: full_moves,
                start_point: starting_point,
            }
        })
}
