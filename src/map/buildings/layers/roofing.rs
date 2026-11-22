//! Кровельная сборка — один разворот на оба режима: заказ формы
//! ([`roofing_of`]), скаты и вальмы, слуховые окна, оборудование кровли.
//! Здесь же вид дома целиком ([`HouseLook`]): кровля и облицовка собираются
//! один раз на дом, а не каждым билдером у себя.

use bevy::color::Mix;
use bevy::prelude::*;

use super::garages::{garage_frame, plain_garage_roof};
use super::walls::{
    WALL_LIT_MIX, WALL_SHADED_MIX, WALL_TOP_LIGHTEN, WallSpan, balcony_house, plan_long_axis,
    wall_colors, wall_frame,
};
use crate::map::buildings::classify::BuildingClass;
use crate::map::buildings::clutter::{flat_roof_items, merlons, push_items, ridge_chimney};
use crate::map::buildings::garages::GarageRun;
use crate::map::buildings::material::{
    RoofLook, WallLook, building_seed, roof_look, run_look, run_wall_look, wall_look,
};
use crate::map::buildings::roofs::{DormerFace, HipRoof, RoofShape, Roofing, TentRoof, roofing_of};
use crate::map::buildings::{HouseFootprint, Lean, shade_by_light};
use crate::map::meshing::{MeshBuilder, WallMark};
use crate::map::osm::PolyArea;

/// Стекло слухового окна, линейный цвет: тёмная комната с отблеском неба.
const DORMER_GLASS: LinearRgba = LinearRgba::rgb(0.05, 0.07, 0.09);

/// Слуховые окна в меш: щёчки и передняя стенка в тоне фасада, стекло, крыша
/// окна в фактуре кровли. Невидимые с камеры стенки не кладутся вовсе —
/// painter's порядок внутри окна тогда держится сам.
fn push_dormers(
    builder: &mut MeshBuilder,
    faces: &[DormerFace],
    look: &RoofLook,
    facade: Srgba,
    lift_dir: Vec2,
) {
    let visible = |normal: Vec2| normal.dot(-lift_dir) > 0.0;
    for face in faces {
        match face {
            DormerFace::Wall(points, normal) if visible(*normal) => {
                builder.set_roof(None);
                let tone = shade_by_light(facade, *normal, WALL_LIT_MIX, WALL_SHADED_MIX)
                    .mix(&Srgba::WHITE, WALL_TOP_LIGHTEN);
                builder.push_convex(points, tone.into());
            }
            DormerFace::Glass(points, normal) if visible(*normal) => {
                builder.set_roof(None);
                builder.push_convex(points, DORMER_GLASS);
            }
            DormerFace::Roof(points, tone) => {
                builder.set_roof(Some(look.frame));
                builder.push_convex(points, *tone);
            }
            _ => {}
        }
    }
    builder.set_roof(Some(look.frame));
}

/// Вальма в меш: скаты по контуру, потом площадка конька поверх них.
fn push_hip(builder: &mut MeshBuilder, roof: &HipRoof) {
    for (slope, tone) in &roof.slopes {
        builder.push_quad(*slope, *tone);
    }
    let (ridge, tone) = &roof.ridge;
    builder.push_polygon(ridge, &[], *tone);
}

/// Концы «конька» вальмы — две самые далёкие друг от друга точки её
/// площадки. У настоящей вальмы конёк это отрезок, и на четырёх-двадцати
/// вершинах площадки перебор пар дешевле любой геометрии.
fn hip_ridge_ends(roof: &HipRoof) -> (Vec2, Vec2) {
    let ring = &roof.ridge.0;
    let mut best = (ring[0], ring[0]);
    let mut span = 0.0;
    for (index, &a) in ring.iter().enumerate() {
        for &b in &ring[index + 1..] {
            let distance = a.distance_squared(b);
            if distance > span {
                span = distance;
                best = (a, b);
            }
        }
    }
    best
}

/// Кровля этого дома: у бокса, вошедшего в гаражный прогон, она берётся
/// **от прогона**, а не от него самого. В этом весь приём: общий посев и
/// общая ось превращают двадцать домиков в одну ленту, а по-своему посеянный
/// бокс красится и ребрится сам по себе.
fn look_of(building: &PolyArea, run: Option<&GarageRun>, fp: Option<HouseFootprint>) -> RoofLook {
    match run {
        Some(run) => run_look(run),
        None => roof_look(building, fp),
    }
}

/// Вид дома целиком — общий шаг сборки для обоих режимов: кровля и облицовка
/// собираются один раз на дом, а не каждым билдером у себя. Облицовка
/// оглядывается на прогон (бокс одет воротами), фасадная полоса — нет: она
/// красится настоящей стеной дома, даже когда бокс стоит в прогоне.
pub(super) struct HouseLook {
    /// Кровля: материал, цвет и рамка фактуры; у бокса прогона — общий вид
    /// всего прогона.
    pub(super) roof: RoofLook,
    /// Облицовка с оглядкой на прогон: бокс одет воротами.
    pub(super) wall: WallLook,
    /// Цвет настоящей стены дома — фасадной полосе плоских режимов: полоса
    /// не ворота, и красит её дом.
    pub(super) facade: Srgba,
}

pub(super) fn house_look(
    building: &PolyArea,
    run: Option<&GarageRun>,
    fp: Option<HouseFootprint>,
) -> HouseLook {
    // вид стены считается один раз: без прогона фасад и облицовка — один и
    // тот же `WallLook`, с прогоном облицовку перекрашивают ворота
    let wall = wall_look(building, fp);
    HouseLook {
        roof: look_of(building, run, fp),
        wall: match run {
            Some(run) => run_wall_look(run),
            None => wall,
        },
        facade: wall.base,
    }
}

/// Плоская кровля: заливка контура (с дворами-дырками) под фактуру своего
/// материала. Ровно это игра кладёт на всякий дом без скатной крыши — и в
/// плоских режимах, где `outer` это сам контур, и в 2.5D, где он уже поднят
/// на высоту стен.
///
/// **Бортика по краю у мягкой кровли больше нет.** Он был каймой в 0.7 м,
/// затенённой по нормали ребра, — то есть той же самой конструкцией, что и
/// скаты вальмы, только уже; с воздуха он читался не парапетом, а маленькой
/// вальмой на каждом панельном доме, и отличить по нему настоящую вальму от
/// плоской крыши было нельзя. Край кровли и без него держат стены под ней.
pub(super) fn push_flat_roof(
    builder: &mut MeshBuilder,
    look: &RoofLook,
    outer: &[Vec2],
    holes: &[Vec<Vec2>],
    color: Srgba,
    lift: Vec2,
) {
    let Some(run) = look.run.as_ref() else {
        builder.set_roof(Some(look.frame));
        builder.push_polygon(outer, holes, color.into());
        return;
    };
    // гаражный контур считается в ячейках, как стена: рамка у каждой вершины
    // своя, и мировая точка с осью шейдеру не нужны. Ячейки эти — **куска**,
    // а не дома, поэтому и кровля кладётся по куску за раз: куски покрывают
    // контур целиком и без нахлёста
    if run.rects.len() > 1 {
        for rect in &run.rects {
            match rect.plain {
                // кривой обрезок: гребёнки боксов на нём нет вовсе, только
                // профлист того же цвета, что у соседей по контуру
                true => builder.set_roof(Some(plain_garage_roof(look))),
                false => builder.set_wall(garage_frame(rect, look.frame.seed, lift)),
            }
            let ring: Vec<Vec2> = rect.ring.iter().map(|at| *at + lift).collect();
            builder.push_polygon(&ring, &[], color.into());
        }
        return;
    }
    // один кусок — значит резать было нечего, и контур кладётся как пришёл:
    // с дворами, которых у куска не бывает
    builder.set_wall(garage_frame(run.main(), look.frame.seed, lift));
    builder.push_polygon(outer, holes, color.into());
}

/// Крыша дома в меш — общий случай сборки, один на оба режима: заказ формы
/// ([`roofing_of`]), фронтоны и слуховые окна у подъёма, оборудование кровли.
/// Дымоход у ската и вальмы, коробки на плоской кровле и зубцы крепости —
/// одна развилка на все дома: раньше её держали два матча (2.5D и плоский),
/// и условия дымоходов в них жили руками.
///
/// Плоский режим — **вырожденный случай без подъёма**: `lean` не задан,
/// конёк не поднимается ([`roofing_of`] получает нулевой подъём), фронтоны и
/// слуховые окна не рисуются — им нужен подъём, в плоскости они лежали бы
/// поверх скатов, — и оборудование кладётся без вдоль-подъёмной тени.
#[allow(clippy::too_many_arguments)]
pub(super) fn push_roofing(
    builder: &mut MeshBuilder,
    building: &PolyArea,
    house: &HouseLook,
    color: Srgba,
    shape: RoofShape,
    clutter: bool,
    class: &BuildingClass,
    lift: Vec2,
    lean: Option<Lean>,
    fp: Option<HouseFootprint>,
) -> RoofShape {
    let seed = building_seed(building);
    let look = &house.roof;
    let facade_color = house.wall.base;
    let storeys = class.storeys();
    // длинная ось плана — только там, где есть что решать фронтона и торцам:
    // без подъёма и их нет
    let long_axis = match lean {
        Some(_) if balcony_house(building, house.wall.kind, storeys) => plan_long_axis(fp),
        _ => None,
    };
    let chimney_on = |builder: &mut MeshBuilder, ridge, ridge_offset| {
        if clutter {
            let chimney: Vec<_> = ridge_chimney(building, look, ridge, lift + ridge_offset)
                .into_iter()
                .collect();
            push_items(builder, &chimney, lean, color);
        }
    };
    let built = match roofing_of(
        shape,
        building,
        class,
        lift,
        |rise| lean.map_or(Vec2::ZERO, |lean| lean.ridge(rise)),
        color,
        seed,
        fp,
    ) {
        Roofing::Gable(roof) => {
            // фронтон — верх торцевой стены, видим по тому же правилу, что
            // и стена под ним: наружная нормаль торца смотрит против подъёма
            if let Some(lean) = lean {
                let lift_dir = lean.dir();
                for ((a, b), face) in &roof.gables {
                    let (a, b) = (*a, *b);
                    let edge = b - a;
                    if Vec2::new(edge.y, -edge.x).dot(-lift_dir) <= 0.0 {
                        continue;
                    }
                    let (_, top) = wall_colors(facade_color, a, b, lift_dir);
                    // фронтон продолжает раму стены под ним — иначе рисунок рвался
                    // бы ровно на карнизе, — но помечен как «над карнизом»: проёмов
                    // на треугольнике нет, окно на нём резалось бы скатом
                    let span = WallSpan::new(a, b, lift, storeys, long_axis);
                    builder.set_wall(
                        wall_frame(building, &house.wall, &span)
                            .map(|frame| frame.marked(WallMark::Solid)),
                    );
                    builder.push_polygon(face, &[], top);
                }
            }
            builder.set_roof(Some(look.frame));
            for (slope, slope_color) in &roof.slopes {
                // скат плоский и выпуклый, и проекция выпуклость сохраняет
                builder.push_convex(slope, *slope_color);
            }
            if let Some(lean) = lean {
                push_dormers(builder, &roof.dormers, look, facade_color, lean.dir());
            }
            if let Some(ridge) = roof.ridge {
                chimney_on(builder, ridge, roof.ridge_offset);
            }
            RoofShape::of_gable(&roof)
        }
        Roofing::Hip(roof) => {
            // у вальмы фронтонов нет — скаты сходятся со всех сторон, и
            // торцевая стена кончается на карнизе, как и боковая
            builder.set_roof(Some(look.frame));
            let ridge = hip_ridge_ends(&roof);
            let ridge_offset = roof.ridge_offset;
            push_hip(builder, &roof);
            chimney_on(builder, ridge, ridge_offset);
            RoofShape::Hip
        }
        Roofing::Tent(roof) => {
            builder.set_roof(Some(look.frame));
            push_tent(builder, &roof);
            RoofShape::Tent
        }
        Roofing::Flat => {
            let roof_outer: Vec<Vec2> = building.outer.iter().map(|p| *p + lift).collect();
            let roof_holes: Vec<Vec<Vec2>> = building
                .holes
                .iter()
                .map(|hole| hole.iter().map(|p| *p + lift).collect())
                .collect();
            push_flat_roof(builder, look, &roof_outer, &roof_holes, color, lift);
            if clutter {
                let items = flat_roof_items(building, look, lift);
                push_items(builder, &items, lean, color);
            }
            RoofShape::Flat
        }
    };
    // зубцы по верху крепостной стены — оборудование этого дома; главы храма
    // кладёт вызывающий, после всех домов (`temples::push_crowns`)
    if clutter {
        push_items(builder, &merlons(building, lift), lean, color);
    }
    built
}

/// Шатёр в меш: грани уже в порядке кладки ([`TentRoof::faces`]).
fn push_tent(builder: &mut MeshBuilder, roof: &TentRoof) {
    for (face, tone) in &roof.faces {
        builder.push_triangle(*face, *tone);
    }
}
