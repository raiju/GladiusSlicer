use geo::*;
use geo_clipper::*;
use geo::prelude::*;
use crate::types::{Command, StateChange, Move, MoveType, MoveChain};
use crate::settings::Settings;
use itertools::{Itertools, chain};
use std::iter::FromIterator;
use std::collections::VecDeque;
use ordered_float::OrderedFloat;

pub struct Slice{
    MainPolygon: MultiPolygon<f64>,
    remaining_area: MultiPolygon<f64>,
    solid_infill: Option<MultiPolygon<f64>>,
    normal_infill: Option<MultiPolygon<f64>>,
    chains: Vec<MoveChain>,
}

impl Slice{
    pub fn from_single_point_loop< I>( line: I) -> Self where I: Iterator<Item = (f64,f64)> {
        let polygon = Polygon::new(
            LineString::from_iter(line) ,
            vec![],
        );

        Slice{MainPolygon: MultiPolygon(vec![polygon.clone()]),remaining_area: MultiPolygon(vec![polygon]),solid_infill:None,normal_infill: None,chains: vec![]}
    }

     pub fn from_multiple_point_loop( lines: MultiLineString<f64>)  -> Self{

         let mut lines_and_area : Vec<(LineString<f64>,f64)> = lines.into_iter().map(|line|{
             let area = line.clone().into_points().iter().circular_tuple_windows::<(_,_)>().map(|(p1,p2)| {
                 (p1.x()+p2.x())*(p2.y()-p1.y())
             }).sum();
             (line,area)
         }).collect();

         lines_and_area.sort_by(|(l1,a1),(l2,a2)| a2.partial_cmp(a1).unwrap());
         let mut polygons = vec![];

         for (line,area) in lines_and_area{
             if area > 0.0{
                 polygons.push(Polygon::new(line.clone(), vec![]));
             }
             else{
                 //counter clockwise interior polygon
                 let smallest_polygon = polygons.iter_mut().rev().find(|poly| poly.contains(&line.0[0] ) ).expect("Polygon order failure");
                 smallest_polygon.interiors_push(line);
             }
         }

        let multi_polygon :MultiPolygon<f64> = MultiPolygon(polygons);

        Slice{MainPolygon: multi_polygon.clone(),remaining_area: multi_polygon,solid_infill:None,normal_infill: None,chains: vec![]}
    }

    pub fn slice_perimeters_into_chains(&mut self,settings : &Settings){
        //Create the outer shells
        for _ in 0..3{
            let (m,mut new_chains) =  inset_polygon(&self.remaining_area,settings);
            self.remaining_area = m;
            self.chains.append(&mut new_chains);
        }

    }

    pub fn fill_remaining_area(&mut self,settings:&Settings, solid: bool, layer_count: usize, layer_height: f64){
        //For each region still available fill wih infill
        for poly in &self.remaining_area
        {
            if solid{

                let angle = (120 as f64) * layer_count as f64;

                let rotate_poly = poly.rotate_around_point(angle,Point(Coordinate::zero()));

                let new_moves = solid_fill_polygon(&rotate_poly,settings);

                if let Some(mut chain) = new_moves{
                    chain.rotate(-angle.to_radians());
                    self.chains.push(chain);
                }

            }
            else{
                let new_moves = partial_fill_polygon(&poly,settings,settings.infill_percentage);

                if let Some(chain) = new_moves{
                    self.chains.push(chain);
                }
            }

        }
    }
    pub fn slice_into_commands(&mut self,settings:&Settings, commands: &mut Vec<Command>) {

        //Order Chains for fastest print
        if self.chains.len() >0 {
            let mut ordered_chains = vec![self.chains.swap_remove(0)];

            while !self.chains.is_empty() {
                let index = self.chains.iter().position_min_by_key(|a| OrderedFloat(ordered_chains.last().unwrap().moves.last().unwrap().end.euclidean_distance(&a.start_point))).unwrap();
                let closest_chain = self.chains.remove(index);
                ordered_chains.push(closest_chain);
            }

            let mut full_moves = vec![];
            let starting_point = ordered_chains[0].start_point;
            for mut chain in ordered_chains {
                full_moves.push(Move { end: chain.start_point, move_type: MoveType::Travel });
                full_moves.append(&mut chain.moves)
            }

            commands.append(&mut MoveChain { moves: full_moves, start_point: starting_point }.create_commands(settings));
        }
    }
}


fn inset_polygon( poly: &MultiPolygon<f64>, settings : &Settings) -> (MultiPolygon<f64>,Vec<MoveChain>){

    let mut move_chains =  vec![];
    let inset_poly = poly.offset(-settings.layer_width/2.0,JoinType::Square,EndType::ClosedPolygon,1000000.0);

    for polygon in inset_poly.0.iter()
    {
        let mut moves = vec![];


        for (&start,&end) in polygon.exterior().0.iter().circular_tuple_windows::<(_,_)>(){
            moves.push(Move{end: end,move_type: MoveType::Outer_Perimeter});
        }

        move_chains.push(MoveChain{start_point:polygon.exterior()[0], moves});

        for interior in polygon.interiors() {
            let mut moves = vec![];
            for (&start, &end) in interior.0.iter().circular_tuple_windows::<(_, _)>() {
                moves.push(Move{end: end,move_type: MoveType::Outer_Perimeter});
            }
            move_chains.push(MoveChain{start_point:interior.0[0], moves});
        }

    }

    (inset_poly.offset(-settings.layer_width/2.0,JoinType::Square,EndType::ClosedPolygon,1000000.0),move_chains)
}

fn solid_fill_polygon( poly: &Polygon<f64>, settings : &Settings) -> Option<MoveChain> {
    let mut moves =  vec![];

    let mut lines : Vec<(Coordinate<f64>,Coordinate<f64>)> = poly.exterior().0.iter().map(|c| *c).circular_tuple_windows::<(_, _)>().collect();

    for interior in poly.interiors(){
        let mut new_lines = interior.0.iter().map(|c| *c).circular_tuple_windows::<(_, _)>().collect();
        lines.append(&mut new_lines);
    }

    for line in lines.iter_mut(){
        *line = if line.0.y < line.1.y {
            *line
        }
        else{
            (line.1,line.0)
        };
    };

    lines.sort_by(|a,b| b.0.y.partial_cmp(&a.0.y).unwrap());

    let mut current_y = lines[lines.len() -1].0.y + settings.layer_width/2.0;

    let mut current_lines = Vec::new();

    let mut orient = false;

    let mut start_point = None;

    let mut line_change = false;

    while !lines.is_empty(){
        line_change = false;
        while !lines.is_empty() && lines[lines.len() -1].0.y < current_y{

            current_lines.push(lines.pop().unwrap());
            line_change = true;
        }


        if lines.is_empty(){
            break;
        }

        current_lines.retain(|(s,e)| e.y > current_y );



        //current_lines.sort_by(|a,b| b.0.x.partial_cmp(&x.0.y).unwrap().then(b.1.x.partial_cmp(&a.1.x).unwrap()) )

        let mut points = current_lines.iter().map(|(start,end)| {
            let x = ((current_y- start.y) * ((end.x - start.x)/(end.y - start.y))) + start.x;
            x
        }).collect::<Vec<_>>();

        points.sort_by(|a,b| a.partial_cmp(b).unwrap());

        start_point = start_point.or(Some(Coordinate{x: points[0], y: current_y}));

        moves.push(Move{ end: Coordinate{x: points[0], y: current_y},move_type: MoveType::Travel});

        if orient {
            for (start, end) in points.iter().tuples::<(_, _)>() {
                moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::Travel} );
                moves.push(Move{ end: Coordinate { x: *end, y: current_y }  ,move_type: MoveType::SolidInfill} );
            }
        }
        else{
            for (start, end) in points.iter().rev().tuples::<(_, _)>() {
                moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::Travel} );
                moves.push(Move{ end: Coordinate { x: *end, y: current_y }  ,move_type: MoveType::SolidInfill} );
            }
        }

        orient = !orient;
        current_y += settings.layer_width;

    }


    start_point.map(|start_point|MoveChain{moves,start_point })

}

fn partial_fill_polygon( poly: &Polygon<f64>, settings : &Settings, fill_ratio: f64) -> Option<MoveChain> {
    let mut moves =  vec![];

    let mut lines : Vec<(Coordinate<f64>,Coordinate<f64>)> = poly.exterior().0.iter().map(|c| *c).circular_tuple_windows::<(_, _)>().collect();

    for interior in poly.interiors(){
        let mut new_lines = interior.0.iter().map(|c| *c).circular_tuple_windows::<(_, _)>().collect();
        lines.append(&mut new_lines);
    }

    for line in lines.iter_mut(){
        *line = if line.0.y < line.1.y {
            *line
        }
        else{
            (line.1,line.0)
        };
    };

    lines.sort_by(|a,b| b.0.y.partial_cmp(&a.0.y).unwrap());

    let distance = settings.layer_width / fill_ratio;

    let mut current_y = (lines[lines.len() -1].0.y / distance).ceil() * distance ;

    let mut current_lines = Vec::new();

    let mut orient = false;

    let mut start_point = None;

    let mut line_change = false;

    let distance = settings.layer_width / fill_ratio;

    while !lines.is_empty(){
        line_change = false;
        while !lines.is_empty() && lines[lines.len() -1].0.y < current_y{

            current_lines.push(lines.pop().unwrap());
            line_change = true;
        }


        if lines.is_empty(){
            break;
        }

        current_lines.retain(|(s,e)| e.y > current_y );



        //current_lines.sort_by(|a,b| b.0.x.partial_cmp(&x.0.y).unwrap().then(b.1.x.partial_cmp(&a.1.x).unwrap()) )

        let mut points = current_lines.iter().map(|(start,end)| {
            let x = ((current_y- start.y) * ((end.x - start.x)/(end.y - start.y))) + start.x;
            x
        }).collect::<Vec<_>>();

        points.sort_by(|a,b| a.partial_cmp(b).unwrap());

        start_point = start_point.or(Some(Coordinate{x: points[0], y: current_y}));

        moves.push(Move{ end: Coordinate{x: points[0], y: current_y},move_type: MoveType::Travel});

        if orient {
            for (start, end) in points.iter().tuples::<(_, _)>() {
                if !line_change{
                    moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::SolidInfill} );
                } else{
                    moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::Travel} );
                }
                moves.push(Move{ end: Coordinate { x: *end, y: current_y }  ,move_type: MoveType::SolidInfill} );
            }
        }
        else{
            for (start, end) in points.iter().rev().tuples::<(_, _)>() {
                if !line_change{
                    moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::SolidInfill} );
                } else{
                    moves.push(Move{ end: Coordinate { x: *start, y: current_y },move_type: MoveType::Travel} );
                }
                moves.push(Move{ end: Coordinate { x: *end, y: current_y }  ,move_type: MoveType::SolidInfill} );
            }
        }

        orient = !orient;
        current_y += distance;

    }


    start_point.map(|start_point|MoveChain{moves,start_point })

}





