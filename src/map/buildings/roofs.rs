//! Скатные крыши малых домов. `roof:shape` в OSM стоит у 283 зданий Тулы из
//! 7465, так что форма крыши не читается из тегов, а **выводится**: частный
//! дом (`BuildingUse::House`) и любая мелкая коробка без назначения получают
//! скатную крышу, остальное — плоскую. Плоская крыша у частного сектора была
//! главной причиной, по которой окраины читались как склад контейнеров.
//!
//! Частный сектор на снимке — двускатный, и выбор формы — контур плюс посев
//! дома ([`roofing`]):
//!
//! * **крыши с фронтонами** ([`GableRoof`], [`GableForm`]) — двускатная,
//!   изредка полувальмовая и ломаная мансардная, у сарая односкатная, у
//!   Г-, Т- и П-образного дома крестовая ([`cross_gable`]); на двускатной
//!   бывает одно слуховое окно;
//! * **двускатная** ([`gable_roof`]) — конёк вдоль длинной оси минимального
//!   описанного прямоугольника (OBB); крыша рисуется по этому прямоугольнику,
//!   а не по контуру, — у настоящего дома скаты и так нависают над стеной.
//!   Поэтому она ставится только на контур, заполняющий прямоугольник почти
//!   целиком ([`RECT_FILL_MIN`]): иначе из дома торчала бы крыша буквой Г, — и
//!   чьи углы лежат на стенах ([`GABLE_OVERHANG_MAX`]): у косого
//!   четырёхугольника угол крыши висит над пустотой.
//! * **вальмовая** ([`HipRoof`]) — скаты по всему контуру и площадка конька
//!   внутри, построенные вдвигом контура на miter-офсетах. Ей форма контура
//!   безразлична, и достаётся она редкому крупному дому и контуру, на котором
//!   не встала ни одна крыша с фронтонами.
//!
//! Те же формы наружу отдаёт [`RoofShape`] — уже как **вход**: город им
//! не пользуется, а витрина `roof_gallery` ставит по нему один контур под
//! всеми крышами разом. Числа, по которым выбор и делается, отдаёт
//! [`shape_facts`]: заполнение описанного прямоугольника и вылет ската вальмы
//! — на картинке вальму от плоской крыши с фаской отличают только они.

use bevy::prelude::*;

use super::shade_by_light;
use crate::map::meshing::{merge_close_points, min_area_rect, miter_offsets};
use crate::map::osm::model::{distance_to_segment, signed_ring_area};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};

/// Какую долю своего описанного прямоугольника контур обязан заполнять,
/// чтобы прямоугольная крыша не торчала из него. Дом с эркером или срезанным
/// углом проходит, Г-образный (≈0.5–0.7) — нет.
const RECT_FILL_MIN: f32 = 0.85;
/// Насколько угол прямоугольной крыши может отстоять от контура, м. Заполнения
/// мало: косой четырёхугольник (Тула, way 968419942, углы 79°–100°) заполняет
/// прямоугольник на 0.91, а угол крыши висит в 2.2 м от стены — под ним нет
/// ничего, и в 2.5D торец дома читается срезанным. Полметра с запасом — это
/// свес карниза и неточность обводки; в Туле дальше него уходят ~550 из
/// 4 800 кандидатов, и им достаётся вальма, которая ложится по самому контуру.
const GABLE_OVERHANG_MAX: f32 = 0.6;
/// Здание без назначения не крупнее этого, м², считается частным домом:
/// в Туле `building=yes` стоит на 4004 контурах из 7465, и за окраины
/// отвечает именно эта половина. Одна граница на крышу, стены и высоту дома
/// (`material`, `heights`).
pub(super) const SMALL_FOOTPRINT_MAX: f32 = 250.0;
/// Подъём конька на метр половины ширины дома — тангенс угла ската
/// (0.8 ≈ 39°). Круче, чем у типовой шиферной крыши: на карте сверху рисуется
/// доля высоты, и пологий скат вовсе не читался бы.
const ROOF_PITCH: f32 = 0.8;
/// Настоящих метров конька над карнизом, не больше: широкий ангар с
/// пятиметровым коньком ещё дом, с десятиметровым — цирк.
const ROOF_RISE_MAX: f32 = 5.0;
/// Насколько скат, повёрнутый к свету, светлее базового тона крыши, а
/// отвёрнутый — темнее. Мягче стен (`WALL_*_MIX`): крыша смотрит в небо и
/// освещена вся, разница только в наклоне. Значения пересчитаны из прежнего
/// смешивания в линейном пространстве, чтобы видимый шаг остался тем же.
const SLOPE_LIT_MIX: f32 = 0.14;
const SLOPE_SHADED_MIX: f32 = 0.11;

/// Крыша с фронтонами, разложенная на куски для painter's algorithm:
/// фронтоны рисуются со стенами, скаты — поверх, слуховые окна — последними.
///
/// Одна структура на все формы, у которых над карнизом остаётся кусок стены:
/// двускатная, полувальмовая, ломаная, односкатная и крестовая над Г-образным
/// домом ([`GableForm`]). Отличаются они только числом и формой граней, а
/// кладутся одинаково.
pub(super) struct GableRoof {
    pub(super) form: GableForm,
    /// Грани стены над карнизом: ребро прямоугольника `(a, b)` на уровне
    /// карниза (CCW, наружная нормаль — правый перпендикуляр) — по нему
    /// берутся рама и видимость, — и сама грань. У двускатной это треугольник
    /// `[a, b, конёк]`, у ломаной пятиугольник, у односкатной ещё и высокая
    /// продольная стена.
    pub(super) gables: Vec<((Vec2, Vec2), Vec<Vec2>)>,
    /// Скаты: выпуклый многоугольник и тон. У двускатной первый скат —
    /// `[карниз, карниз, конёк, конёк]`.
    pub(super) slopes: Vec<(Vec<Vec2>, LinearRgba)>,
    /// Слуховые окна (дормеры) — поверх скатов, грани в порядке кладки.
    pub(super) dormers: Vec<DormerFace>,
    /// Концы конька в нарисованных координатах; у односкатной конька нет.
    pub(super) ridge: Option<(Vec2, Vec2)>,
    /// Сдвиг конька над карнизом — тот самый `ridge_lift(подъём)`, которым
    /// подняты вершины конька. Нужен всякому, кто ставит предмет на конёк и
    /// должен вернуть его в систему настоящего контура.
    pub(super) ridge_offset: Vec2,
}

/// Какая из крыш с фронтонами стоит над домом. Частный сектор на снимке —
/// это двускатные крыши с редкими полувальмами и ломаными мансардами, и
/// Г-образный дом кроется не вальмой, а двумя двускатными с ендовой.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum GableForm {
    Gable,
    /// Двускатная, у которой верх фронтона срезан маленькой вальмой.
    HalfHip,
    /// Ломаная (мансардная): крутой нижний скат и пологий верхний — под ней
    /// жилой чердак, и дом в один этаж выглядит полутораэтажным.
    Gambrel,
    /// Односкатная — сараи, бани и пристройки во дворах.
    LeanTo,
    /// Две двускатные над Г-образным домом: крыло упирается коньком в скат
    /// основного корпуса.
    Cross,
}

/// Грань слухового окна. Тон стены и стекла слой решает сам — он знает цвет
/// фасада, — поэтому им отдаётся наружная нормаль в плане.
pub(super) enum DormerFace {
    Wall(Vec<Vec2>, Vec2),
    Glass(Vec<Vec2>, Vec2),
    Roof(Vec<Vec2>, LinearRgba),
}

/// Вальмовая крыша: скаты по **всему** контуру и площадка конька внутри.
///
/// Строится не straight skeleton'ом, а вдвигом контура внутрь на
/// [`HIP_INSET`] теми же miter-офсетами, что дают дальний край каймы: скат —
/// квад между ребром контура и его сдвинутой парой, конёк — то, что осталось
/// внутри. Для выпуклого дома это и есть вальма; для Г-образного — вальма с
/// плоской верхушкой, то есть ровно то, что видно на снимке, и то, чего
/// straight skeleton стоил бы на порядок дороже.
pub(super) struct HipRoof {
    /// Скаты: четырёхугольник (карниз, карниз, конёк, конёк) и тон, по одному
    /// на ребро контура.
    pub(super) slopes: Vec<([Vec2; 4], LinearRgba)>,
    /// Площадка конька — вдвинутый контур и его тон.
    pub(super) ridge: (Vec<Vec2>, LinearRgba),
    /// Сдвиг конька над карнизом — тот самый `ridge_lift(ridge_rise(inset))`,
    /// которым подняты вершины скатов. Нужен всякому, кто ставит предмет на
    /// конёк и должен вернуть его в систему настоящего контура.
    pub(super) ridge_offset: Vec2,
}

/// Шатёр: грани от каждого ребра контура к одной вершине над центром. Так
/// кроются башни — крепостная ([`super::fortress`]) и колокольня, у которой
/// шатёр вытянут в шпиль ([`super::temples`]).
pub(super) struct TentRoof {
    /// Грани в порядке кладки: сперва отвёрнутые от камеры, потом обращённые
    /// к ней, — шатёр выпуклый, и этого painter's порядка ему достаточно.
    pub(super) faces: Vec<([Vec2; 3], LinearRgba)>,
}

/// Что за крыша у дома. Плоская — не «крыши нет», а именно плоская кровля со
/// своим материалом, фактурой и оборудованием.
pub(super) enum Roofing {
    Gable(GableRoof),
    Hip(HipRoof),
    Tent(TentRoof),
    Flat,
}

/// Форма крыши, которую дому **назначает его смысл**, а не вывод по контуру и
/// посеву: храм и крепость кроются так, как кроются храм и крепость, какого бы
/// размера ни были. Решают [`super::temples::roof_form`] и
/// [`super::fortress::roof_form`].
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum LandmarkRoof {
    Flat,
    Hip,
    /// Крутая двускатная — костёл и кирха. Не встала (контур не прямоугольный)
    /// — вальма.
    SteepGable,
    /// Шатёр высотой в `rise` сторон плана (сторона — корень из площади).
    Tent {
        rise: f32,
    },
}

/// Подъём конька крутой двускатной на метр половины ширины: 1.3 ≈ 52°, готика
/// и кирха круче жилого дома.
const STEEP_PITCH: f32 = 1.3;
/// Потолок подъёма крутого конька, м.
const STEEP_RISE_MAX: f32 = 12.0;
/// Потолок подъёма шатра, м: шпиль выше сорока метров над карнизом на карте
/// сверху ложится поперёк квартала.
const TENT_RISE_MAX: f32 = 40.0;
/// Насколько светлее и темнее базового тона грань шатра по свету — круче
/// ската, поэтому контраст сильнее, чем у двускатной.
const TENT_LIT_MIX: f32 = 0.20;
const TENT_SHADED_MIX: f32 = 0.22;

/// Вылет ската вальмы по плану, м, и потолок этого вылета в долях толщины
/// контура (`площадь / периметр`): у узкого дома скаты обязаны сойтись, а не
/// вывернуться наизнанку — тот же зажим, что у каймы.
const HIP_INSET: f32 = 2.2;
const HIP_INSET_SHARE: f32 = 0.38;
/// Насколько площадка конька светлее базового тона: она смотрит прямо в небо,
/// а скаты — вбок.
const RIDGE_LIGHTEN: f32 = 0.06;

/// Какую форму крыши положить дому, минуя вывод по посеву.
///
/// Игре она не нужна: форму дома игра выводит сама ([`roofing`]) — по
/// назначению, контуру и посеву. Заказывает форму витрина `roof_gallery`,
/// которой нужен **один и тот же контур под всеми тремя**: разница между
/// двускатной и вальмовой на доме одного размера иначе не видна, а «игра
/// откажется строить здесь двускатную» не отличить от «она её просто не
/// выбрала».
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoofShape {
    /// Как решит игра.
    Auto,
    Flat,
    Gable,
    Hip,
    /// Шатёр — башни крепости и колокольни. Выводом по посеву жилому дому не
    /// достаётся никогда, только назначением ([`LandmarkRoof::Tent`]).
    Tent,
    /// Полувальмовая ([`GableForm::HalfHip`]).
    HalfHip,
    /// Ломаная мансардная ([`GableForm::Gambrel`]).
    Gambrel,
    /// Односкатная ([`GableForm::LeanTo`]).
    LeanTo,
    /// Крестовая над Г-образным домом ([`GableForm::Cross`]).
    Cross,
    /// Двускатная со слуховыми окнами.
    Dormer,
}

impl RoofShape {
    /// Какая форма легла в меш у этой крыши с фронтонами.
    pub(super) fn of_gable(roof: &GableRoof) -> Self {
        match roof.form {
            GableForm::Gable if !roof.dormers.is_empty() => Self::Dormer,
            GableForm::Gable => Self::Gable,
            GableForm::HalfHip => Self::HalfHip,
            GableForm::Gambrel => Self::Gambrel,
            GableForm::LeanTo => Self::LeanTo,
            GableForm::Cross => Self::Cross,
        }
    }
}

/// Подъём шатра, заказанного витриной, в сторонах плана.
const GALLERY_TENT_RISE: f32 = 1.0;

/// Числа, по которым выбирается форма крыши, — и по которым видно, что
/// именно получилось.
///
/// Игра принимает по ним решение внутри [`gable_roof`] и [`hip_roof`] и
/// наружу их не отдаёт; витрина печатает их под домом, потому что глазом
/// «вальма» от «плоской крыши с фаской» отличается ровно вылетом ската в
/// метрах, а разная высота силуэта двух домов одного размера — разницей
/// подъёмов конька.
pub struct ShapeFacts {
    /// Доля описанного прямоугольника, занятая контуром: двускатная крыша
    /// требует не меньше [`RECT_FILL_MIN`].
    pub rect_fill: f32,
    /// Дальше всего отстоящий от контура угол описанного прямоугольника, м:
    /// двускатная требует не больше [`GABLE_OVERHANG_MAX`].
    pub gable_overhang: f32,
    /// Подъём конька двускатной над карнизом, настоящих метров.
    pub gable_rise: f32,
    /// Вылет ската вальмы по плану, м; `None` — контур слишком тонкий, и
    /// вальма на нём не встанет.
    pub hip_inset: Option<f32>,
    /// Подъём конька вальмовой над карнизом, настоящих метров.
    pub hip_rise: Option<f32>,
}

/// Числа выбора формы для этого контура; `None` — вырожденное кольцо.
pub fn shape_facts(building: &PolyArea) -> Option<ShapeFacts> {
    let (rect, rect_fill) = bounding_rect(&building.outer)?;
    let hip_inset = hip_plan(&building.outer).map(|(_, inset)| inset);
    Some(ShapeFacts {
        rect_fill,
        gable_overhang: overhang(&building.outer, &rect),
        gable_rise: ridge_rise((rect[2] - rect[1]).length()),
        hip_inset,
        hip_rise: hip_inset.map(hip_rise),
    })
}

/// Крыша заказанной формы.
///
/// От [`roofing`] отличается тем, что **отказ не подменяется**: если на этом
/// контуре двускатная не встаёт, возвращается плоская, а не вальма. Витрина
/// обязана показать сам отказ — подмена выглядела бы как «игра здесь всё-таки
/// строит скат», чего на самом деле нет.
pub(super) fn roofing_of(
    shape: RoofShape,
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    seed: u32,
) -> Roofing {
    match shape {
        RoofShape::Auto => roofing(building, lift, ridge_lift, base, seed),
        RoofShape::Flat => Roofing::Flat,
        RoofShape::Gable => {
            gable_roof(building, lift, &ridge_lift, base).map_or(Roofing::Flat, Roofing::Gable)
        }
        RoofShape::Hip => {
            hip_roof(building, lift, &ridge_lift, base).map_or(Roofing::Flat, Roofing::Hip)
        }
        RoofShape::Tent => tent_roof(building, lift, &ridge_lift, base, GALLERY_TENT_RISE)
            .map_or(Roofing::Flat, Roofing::Tent),
        RoofShape::HalfHip | RoofShape::Gambrel | RoofShape::LeanTo | RoofShape::Dormer => {
            let form = match shape {
                RoofShape::HalfHip => GableForm::HalfHip,
                RoofShape::Gambrel => GableForm::Gambrel,
                RoofShape::LeanTo => GableForm::LeanTo,
                _ => GableForm::Gable,
            };
            let choice = HouseRoof {
                form,
                hipped: false,
                dormers: shape == RoofShape::Dormer,
                seed,
            };
            house_gable(building, lift, &ridge_lift, base, choice)
                .map_or(Roofing::Flat, Roofing::Gable)
        }
        RoofShape::Cross => match is_pitched(building) {
            true => {
                cross_gable(building, lift, &ridge_lift, base).map_or(Roofing::Flat, Roofing::Gable)
            }
            false => Roofing::Flat,
        },
    }
}

/// Крыша дома целиком.
///
/// Частный сектор на снимке — это **двускатные** крыши: полувальма и ломаная
/// мансарда встречаются среди них вкраплениями, сарай во дворе крыт одним
/// скатом, а вальма и плоская кровля у частного дома почти не бывают. Отсюда и
/// порядок: прямоугольному дому форму выбирает посев ([`house_roof`]),
/// Г-образному — крестовая двускатная ([`cross_gable`]), и только прочим
/// контурам, на которых фронтону негде встать, достаётся вальма.
pub(super) fn roofing(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    seed: u32,
) -> Roofing {
    if let Some(form) = landmark_roof(building) {
        return landmark_roofing(form, building, lift, ridge_lift, base);
    }
    if !is_pitched(building) {
        return Roofing::Flat;
    }
    if let Some(rect) = gable_rect(&building.outer) {
        let choice = house_roof(building, &rect, seed);
        // редкая вальма — только большому дому, и только если посев так решил
        if choice.hipped
            && let Some(hip) = hip_roof(building, lift, &ridge_lift, base)
        {
            return Roofing::Hip(hip);
        }
        if let Some(roof) = house_gable(building, lift, &ridge_lift, base, choice) {
            return Roofing::Gable(roof);
        }
    }
    if let Some(roof) = cross_gable(building, lift, &ridge_lift, base) {
        return Roofing::Gable(roof);
    }
    match hip_roof(building, lift, &ridge_lift, base) {
        Some(roof) => Roofing::Hip(roof),
        // на совсем узком контуре вальма выворачивается, а двускатная не
        // встала — пусть будет плоской, это по крайней мере не врёт
        None => gable_roof(building, lift, &ridge_lift, base).map_or(Roofing::Flat, Roofing::Gable),
    }
}

/// Разброс форм крыш скатной когорты одной строкой — как `height_mix` у
/// высот: форма крыши на общем плане глазом не читается, а доля вальм в
/// частном секторе — ровно то, за чем выбор формы и следит.
///
/// Считается по тем формам, что **легли в меш** 2.5D ([`RoofMix::add`] зовёт
/// слой экструзии), а не отдельным проходом: второй вызов [`roofing`] на пять
/// тысяч домов стоил 14 мс на каждую пересборку слоя.
#[derive(Default)]
pub(super) struct RoofMix {
    // gable, dormers, half-hip, gambrel, lean-to, cross, hip, flat
    counts: [usize; 8],
}

impl RoofMix {
    /// Учесть дом, если он из скатной когорты: храмы и корпуса — не про неё.
    pub(super) fn add(&mut self, building: &PolyArea, drawn: RoofShape) {
        if !is_pitched(building) || landmark_roof(building).is_some() {
            return;
        }
        let slot = match drawn {
            RoofShape::Dormer => 1,
            RoofShape::HalfHip => 2,
            RoofShape::Gambrel => 3,
            RoofShape::LeanTo => 4,
            RoofShape::Cross => 5,
            RoofShape::Hip => 6,
            RoofShape::Flat | RoofShape::Tent | RoofShape::Auto => 7,
            RoofShape::Gable => 0,
        };
        self.counts[slot] += 1;
    }
}

impl std::fmt::Display for RoofMix {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.counts.iter().sum::<usize>().max(1);
        let share = |slot: usize| self.counts[slot] * 100 / total;
        write!(
            f,
            "{total} pitched: gable {}% (+dormers {}%), half-hip {}%, gambrel {}%, lean-to {}%, cross {}%, hip {}%, flat {}%",
            share(0),
            share(1),
            share(2),
            share(3),
            share(4),
            share(5),
            share(6),
            share(7),
        )
    }
}

/// Назначенная форма крыши храма или крепости; `None` — обычный дом, форму
/// ему выводит [`roofing`].
pub(super) fn landmark_roof(building: &PolyArea) -> Option<LandmarkRoof> {
    if building.kind == AreaKind::Kremlin {
        return Some(super::fortress::roof_form(building));
    }
    match building.building_use {
        BuildingUse::Church(sacred) => Some(super::temples::roof_form(sacred, building)),
        _ => None,
    }
}

/// Назначенная крыша, с отступлением на ступень проще там, где контур её не
/// держит: крутая двускатная → вальма → плоская, шатёр → плоская.
fn landmark_roofing(
    form: LandmarkRoof,
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Roofing {
    let hip = |ridge_lift: &dyn Fn(f32) -> Vec2| {
        hip_roof(building, lift, ridge_lift, base).map_or(Roofing::Flat, Roofing::Hip)
    };
    match form {
        LandmarkRoof::Flat => Roofing::Flat,
        LandmarkRoof::Hip => hip(&ridge_lift),
        LandmarkRoof::SteepGable => {
            match gable_over(
                building,
                lift,
                &ridge_lift,
                base,
                STEEP_PITCH,
                STEEP_RISE_MAX,
            ) {
                Some(roof) => Roofing::Gable(roof),
                None => hip(&ridge_lift),
            }
        }
        LandmarkRoof::Tent { rise } => {
            tent_roof(building, lift, &ridge_lift, base, rise).map_or(Roofing::Flat, Roofing::Tent)
        }
    }
}

/// Настоящих метров от карниза до верха назначенной крыши: на площадку
/// вальмы и на вершину шатра встают главы храма (`temples.rs`). Считается
/// теми же отступлениями, что и сама крыша ([`landmark_roofing`]), — иначе
/// глава повисла бы над кровлей или утонула в ней. Обычный дом — ноль.
pub(super) fn landmark_rise(building: &PolyArea) -> f32 {
    let hip = || hip_plan(&building.outer).map_or(0.0, |(_, inset)| hip_rise(inset));
    match landmark_roof(building) {
        None | Some(LandmarkRoof::Flat) => 0.0,
        Some(LandmarkRoof::Hip) => hip(),
        Some(LandmarkRoof::SteepGable) => match gable_rect(&building.outer) {
            Some(rect) => pitched_rise((rect[2] - rect[1]).length(), STEEP_PITCH, STEEP_RISE_MAX),
            None => hip(),
        },
        Some(LandmarkRoof::Tent { rise }) => {
            let ring = merge_close_points(&building.outer, true, TENT_MERGE);
            if ring.len() < 3 {
                return 0.0;
            }
            tent_rise(signed_ring_area(&ring).abs().sqrt(), rise)
        }
    }
}

/// Настоящих метров от карниза до вершины шатра над планом со стороной `side`
/// (корень из площади), поднятого на `rise` сторон.
fn tent_rise(side: f32, rise: f32) -> f32 {
    (side * rise).min(TENT_RISE_MAX)
}

/// Вершины контура ближе этого сливаются перед шатром, м.
const TENT_MERGE: f32 = 0.2;

/// Шатёр над контуром: вершина над средним вершин кольца, поднятая на
/// `rise` сторон плана. `None` — вырожденное кольцо.
fn tent_roof(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
) -> Option<TentRoof> {
    let ring = merge_close_points(&building.outer, true, TENT_MERGE);
    if ring.len() < 3 {
        return None;
    }
    let area = signed_ring_area(&ring);
    let side = area.abs().sqrt();
    if side <= 0.0 {
        return None;
    }
    let orientation = area.signum();
    let center = ring.iter().copied().sum::<Vec2>() / ring.len() as f32;
    let ridge_offset = ridge_lift(tent_rise(side, rise));
    let apex = center + lift + ridge_offset;
    // отвёрнутые от камеры грани кладутся первыми: камера смотрит против
    // подъёма, и дальняя сторона шатра обязана оказаться под ближней
    let toward_camera = -ridge_lift(1.0).normalize_or_zero();
    let mut faces: Vec<(f32, [Vec2; 3], LinearRgba)> = (0..ring.len())
        .map(|index| {
            let (a, b) = (ring[index], ring[(index + 1) % ring.len()]);
            let edge = b - a;
            let outward = Vec2::new(edge.y, -edge.x).normalize_or_zero() * orientation;
            let tone = shade_by_light(base, outward, TENT_LIT_MIX, TENT_SHADED_MIX);
            (
                outward.dot(toward_camera),
                [a + lift, b + lift, apex],
                tone.into(),
            )
        })
        .collect();
    faces.sort_by(|x, y| x.0.total_cmp(&y.0));
    Some(TentRoof {
        faces: faces
            .into_iter()
            .map(|(_, face, tone)| (face, tone))
            .collect(),
    })
}

/// Вальмовая крыша над контуром, поднятым на `lift`. `None` — контур слишком
/// тонкий, чтобы скаты сошлись.
fn hip_roof(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<HipRoof> {
    let (ring, inset) = hip_plan(&building.outer)?;
    let area = signed_ring_area(&ring);
    // офсеты смотрят влево по ходу обхода: у CCW-кольца это внутрь
    let side = if area > 0.0 { 1.0 } else { -1.0 };
    let offsets = miter_offsets(&ring, true, inset);
    let rise = ridge_lift(hip_rise(inset));
    let inner: Vec<Vec2> = ring
        .iter()
        .zip(&offsets)
        .map(|(point, offset)| *point + *offset * side + lift + rise)
        .collect();

    let mut slopes = Vec::with_capacity(ring.len());
    for index in 0..ring.len() {
        let next = (index + 1) % ring.len();
        let (a, b) = (ring[index] + lift, ring[next] + lift);
        let outward = Vec2::new((b - a).y, -(b - a).x).normalize_or_zero() * side;
        let tone = shade_by_light(base, outward, SLOPE_LIT_MIX, SLOPE_SHADED_MIX);
        slopes.push(([a, b, inner[next], inner[index]], tone.into()));
    }
    Some(HipRoof {
        slopes,
        ridge: (inner, base.mix(&Srgba::WHITE, RIDGE_LIGHTEN).into()),
        ridge_offset: rise,
    })
}

/// Тоньше этого вдвиг не имеет смысла: скат в двадцать сантиметров не виден,
/// а вершин на контур столько же.
const MIN_HIP_INSET: f32 = 0.4;

/// Кольцо вальмы и вылет её ската, м: контур со слитыми близкими вершинами и
/// вдвиг, зажатый долей толщины контура. `None` — вершин меньше трёх или
/// контур слишком тонкий, чтобы скаты сошлись.
///
/// Отдельной функцией, потому что вылет — это и половина решения о форме
/// ([`shape_facts`] печатает его в витрине), и геометрия ската: посчитанный
/// дважды, он разошёлся бы в первую же правку зажима.
fn hip_plan(outer: &[Vec2]) -> Option<(Vec<Vec2>, f32)> {
    let ring = merge_close_points(outer, true, HIP_INSET / 4.0);
    if ring.len() < 3 {
        return None;
    }
    let inset = HIP_INSET.min(HIP_INSET_SHARE * signed_ring_area(&ring).abs() / perimeter(&ring));
    (inset >= MIN_HIP_INSET).then_some((ring, inset))
}

/// Подъём конька вальмы над карнизом: тот же скат, что у двускатной, только
/// разложенный на вылет каймы, а не на половину ширины дома.
fn hip_rise(inset: f32) -> f32 {
    (inset * ROOF_PITCH).min(ROOF_RISE_MAX)
}

/// Периметр замкнутого контура.
fn perimeter(ring: &[Vec2]) -> f32 {
    (0..ring.len())
        .map(|index| ring[index].distance(ring[(index + 1) % ring.len()]))
        .sum()
}

/// Дом из **скатной когорты**? Предикат гейтит не два ската, а скатную
/// крышу вообще: его `false` — это `Roofing::Flat`, а `true` открывает и
/// двускатную, и вальмовую, причём вальма достаётся в том числе тем, кому
/// двускатная отказала (Г-образный контур).
pub(super) fn is_pitched(building: &PolyArea) -> bool {
    // Кремль вне стилизации по назначению, как и в `base_colors`
    if building.kind == AreaKind::Kremlin {
        return false;
    }
    if !building.holes.is_empty() {
        return false;
    }
    match building.building_use {
        BuildingUse::House => true,
        BuildingUse::Other => signed_ring_area(&building.outer).abs() <= SMALL_FOOTPRINT_MAX,
        _ => false,
    }
}

/// Настоящих метров конька над карнизом для дома шириной `width`.
pub(super) fn ridge_rise(width: f32) -> f32 {
    pitched_rise(width, ROOF_PITCH, ROOF_RISE_MAX)
}

/// Подъём конька двускатной крыши шириной `width` при крутизне `pitch` (метр
/// подъёма на метр половины ширины), не выше `rise_max`.
fn pitched_rise(width: f32, pitch: f32, rise_max: f32) -> f32 {
    (width / 2.0 * pitch).min(rise_max)
}

/// Описанный прямоугольник контура и доля его площади, занятая контуром.
/// `None` — вырожденное кольцо. Вторая половина решения о форме, рядом с
/// [`hip_plan`] и по той же причине: витрина печатает ровно то число, по
/// которому игра отказывает двускатной, а не пересчитанное своё.
fn bounding_rect(ring: &[Vec2]) -> Option<([Vec2; 4], f32)> {
    let rect = min_area_rect(ring)?;
    let area = (rect[1] - rect[0]).length() * (rect[2] - rect[1]).length();
    if area <= 0.0 {
        return None;
    }
    Some((rect, signed_ring_area(ring).abs() / area))
}

/// Как далеко от контура, м, самый дальний угол прямоугольника `rect`.
fn overhang(ring: &[Vec2], rect: &[Vec2; 4]) -> f32 {
    rect.iter()
        .map(|corner| {
            (0..ring.len())
                .map(|index| {
                    distance_to_segment(*corner, ring[index], ring[(index + 1) % ring.len()])
                })
                .fold(f32::INFINITY, f32::min)
        })
        .fold(0.0, f32::max)
}

/// Прямоугольник, над которым встаёт двускатная крыша; `None` — не встаёт:
/// контур заполняет его хуже [`RECT_FILL_MIN`] или какой-то его угол висит
/// дальше [`GABLE_OVERHANG_MAX`] от стены. Крыша рисуется по прямоугольнику,
/// стены — по контуру, и на пустом углу из-под крыши не выходит ни одна стена.
fn gable_rect(ring: &[Vec2]) -> Option<[Vec2; 4]> {
    let (rect, fill) = bounding_rect(ring)?;
    (fill >= RECT_FILL_MIN && overhang(ring, &rect) <= GABLE_OVERHANG_MAX).then_some(rect)
}

/// Двускатная крыша над контуром, поднятым на `lift`; `ridge_lift` — на
/// сколько выше карниза нарисован конёк (в плоских режимах — ноль, и скаты
/// отличаются только тоном). `None` — крыша остаётся плоской: дом не из
/// тех, что [`is_pitched`], или контур не прямоугольный.
pub(super) fn gable_roof(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    if !is_pitched(building) {
        return None;
    }
    gable_over(building, lift, ridge_lift, base, ROOF_PITCH, ROOF_RISE_MAX)
}

/// Двускатная крыша заданной крутизны — без вопроса, положена ли она дому:
/// жилому дому это решает [`is_pitched`], храму — его вера.
fn gable_over(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    pitch: f32,
    rise_max: f32,
) -> Option<GableRoof> {
    let rect = gable_rect(&building.outer)?;
    Some(plain_gable(
        &Frame::of(rect, lift),
        &ridge_lift,
        base,
        pitched_rise((rect[2] - rect[1]).length(), pitch, rise_max),
    ))
}

/// Прямоугольник крыши, уже поднятый на карниз: углы CCW, `c0→c1` — вдоль
/// конька.
struct Frame {
    c: [Vec2; 4],
    /// Единичный вектор вдоль конька.
    long: Vec2,
    length: f32,
    width: f32,
}

impl Frame {
    fn of(rect: [Vec2; 4], lift: Vec2) -> Self {
        let c = rect.map(|corner| corner + lift);
        Self {
            c,
            long: (c[1] - c[0]).normalize_or_zero(),
            length: (c[1] - c[0]).length(),
            width: (c[2] - c[1]).length(),
        }
    }

    /// Наружная нормаль ската у карниза `c0–c1` — правый перпендикуляр к
    /// `c0→c1`; противоположный скат смотрит ровно наоборот.
    fn outward(&self) -> Vec2 {
        Vec2::new(self.long.y, -self.long.x)
    }

    /// Середины торцов на уровне карниза: у `c0–c3` и у `c1–c2`.
    fn mids(&self) -> (Vec2, Vec2) {
        let [c0, c1, c2, c3] = self.c;
        ((c0 + c3) / 2.0, (c1 + c2) / 2.0)
    }
}

/// Тон ската по его наружной нормали; `strength` — во сколько раз крутизна
/// ската резче обычной (крутой низ ломаной крыши темнеет и светлеет сильнее).
fn slope_tone(base: Srgba, outward: Vec2, strength: f32) -> LinearRgba {
    shade_by_light(
        base,
        outward,
        SLOPE_LIT_MIX * strength,
        SLOPE_SHADED_MIX * strength,
    )
    .into()
}

/// Обычная двускатная с подъёмом конька `rise` настоящих метров.
fn plain_gable(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let ridge = ridge_lift(rise);
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let outward = frame.outward();
    GableRoof {
        form: GableForm::Gable,
        slopes: vec![
            (vec![c0, c1, r1, r0], slope_tone(base, outward, 1.0)),
            (vec![r0, r1, c2, c3], slope_tone(base, -outward, 1.0)),
        ],
        gables: vec![((c1, c2), vec![c1, c2, r1]), ((c3, c0), vec![c3, c0, r0])],
        dormers: Vec::new(),
        ridge: Some((r0, r1)),
        ridge_offset: ridge,
    }
}

/// На какой доле высоты фронтона полувальма срезает его верх: ниже — стена,
/// выше — маленький скат.
const HALF_HIP_SPLIT: f32 = 0.55;

/// Полувальмовая: двускатная, у которой верх каждого фронтона срезан скатом
/// той же крутизны. `None` — дом слишком короткий, и срезы сошлись бы.
fn half_hip(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
) -> Option<GableRoof> {
    let [c0, c1, c2, c3] = frame.c;
    let t = HALF_HIP_SPLIT;
    // скат среза той же крутизны, что и боковой: по плану он уходит внутрь
    // на ту же долю полуширины, какую долю высоты он занимает
    let inset = (1.0 - t) * frame.width / 2.0;
    if 2.0 * inset >= frame.length - 1.0 {
        return None;
    }
    let ridge = ridge_lift(rise);
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let (s0, s1) = (r0 + frame.long * inset, r1 - frame.long * inset);
    // точки на скосах фронтона: линейная смесь в нарисованных координатах —
    // та же смесь плана и высоты, проекция аффинна
    let (p0, p1) = (c0.lerp(r0, t), c1.lerp(r1, t));
    let (p2, p3) = (c2.lerp(r1, t), c3.lerp(r0, t));
    let outward = frame.outward();
    Some(GableRoof {
        form: GableForm::HalfHip,
        slopes: vec![
            (vec![c0, c1, p1, s1, s0, p0], slope_tone(base, outward, 1.0)),
            (
                vec![c2, c3, p3, s0, s1, p2],
                slope_tone(base, -outward, 1.0),
            ),
            (vec![p1, p2, s1], slope_tone(base, frame.long, 1.2)),
            (vec![p3, p0, s0], slope_tone(base, -frame.long, 1.2)),
        ],
        gables: vec![
            ((c1, c2), vec![c1, c2, p2, p1]),
            ((c3, c0), vec![c3, c0, p0, p3]),
        ],
        dormers: Vec::new(),
        ridge: Some((s0, s1)),
        ridge_offset: ridge,
    })
}

/// Ломаная мансардная крыша: доля полуширины до излома, крутизна нижнего и
/// верхнего ската и потолок подъёма, настоящих метров. Нижний скат круче
/// жилой двускатной вдвое с лишним — это он делает под крышей комнату.
const GAMBREL_KNEE: f32 = 0.32;
const GAMBREL_LOWER_PITCH: f32 = 2.0;
const GAMBREL_UPPER_PITCH: f32 = 0.5;
const GAMBREL_RISE_MAX: f32 = 6.0;

/// Ломаная (мансардная) крыша: у каждого ската излом, фронтон пятиугольный.
fn gambrel(frame: &Frame, ridge_lift: impl Fn(f32) -> Vec2, base: Srgba) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let half = frame.width / 2.0;
    let knee = GAMBREL_KNEE * half * GAMBREL_LOWER_PITCH;
    let top = knee + (1.0 - GAMBREL_KNEE) * half * GAMBREL_UPPER_PITCH;
    let scale = (GAMBREL_RISE_MAX / top).min(1.0);
    let (knee_lift, ridge) = (ridge_lift(knee * scale), ridge_lift(top * scale));
    let (m0, m1) = frame.mids();
    let (r0, r1) = (m0 + ridge, m1 + ridge);
    let k = GAMBREL_KNEE;
    let (k0, k1) = (c0.lerp(m0, k) + knee_lift, c1.lerp(m1, k) + knee_lift);
    let (k2, k3) = (c2.lerp(m1, k) + knee_lift, c3.lerp(m0, k) + knee_lift);
    let outward = frame.outward();
    GableRoof {
        form: GableForm::Gambrel,
        slopes: vec![
            (vec![c0, c1, k1, k0], slope_tone(base, outward, 1.5)),
            (vec![k0, k1, r1, r0], slope_tone(base, outward, 0.6)),
            (vec![r0, r1, k2, k3], slope_tone(base, -outward, 0.6)),
            (vec![k3, k2, c2, c3], slope_tone(base, -outward, 1.5)),
        ],
        gables: vec![
            ((c1, c2), vec![c1, c2, k2, r1, k1]),
            ((c3, c0), vec![c3, c0, k0, r0, k3]),
        ],
        dormers: Vec::new(),
        ridge: Some((r0, r1)),
        ridge_offset: ridge,
    }
}

/// Односкатная крыша сарая: подъём на метр ширины и его потолок, м.
const LEAN_TO_PITCH: f32 = 0.3;
const LEAN_TO_RISE_MAX: f32 = 2.0;

/// Односкатная: одна плоскость от низкой продольной стены к высокой; под
/// высоким краем встают треугольники торцов и полоса продольной стены.
/// `high_far` — поднят ли край `c2–c3` (иначе `c0–c1`).
fn lean_to(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    high_far: bool,
) -> GableRoof {
    let [c0, c1, c2, c3] = frame.c;
    let up = ridge_lift((frame.width * LEAN_TO_PITCH).min(LEAN_TO_RISE_MAX));
    let outward = frame.outward();
    let (slope, tone, gables) = match high_far {
        true => (
            vec![c0, c1, c2 + up, c3 + up],
            slope_tone(base, outward, 1.0),
            vec![
                ((c1, c2), vec![c1, c2, c2 + up]),
                ((c3, c0), vec![c3, c0, c3 + up]),
                ((c2, c3), vec![c2, c3, c3 + up, c2 + up]),
            ],
        ),
        false => (
            vec![c0 + up, c1 + up, c2, c3],
            slope_tone(base, -outward, 1.0),
            vec![
                ((c1, c2), vec![c1, c2, c1 + up]),
                ((c3, c0), vec![c3, c0, c0 + up]),
                ((c0, c1), vec![c0, c1, c1 + up, c0 + up]),
            ],
        ),
    };
    GableRoof {
        form: GableForm::LeanTo,
        slopes: vec![(slope, tone)],
        gables,
        dormers: Vec::new(),
        ridge: None,
        ridge_offset: Vec2::ZERO,
    }
}

/// Выбор крыши для прямоугольного дома скатной когорты.
#[derive(Clone, Copy)]
pub(super) struct HouseRoof {
    form: GableForm,
    /// Редкая вальма вместо всего остального.
    hipped: bool,
    dormers: bool,
    seed: u32,
}

/// Пятно, до которого дом без назначения считается сараем, баней или летней
/// кухней, м²: такой кроется одним скатом и стоит в один низкий этаж.
pub(super) const SHED_FOOTPRINT_MAX: f32 = 40.0;
/// Формы крыши частного дома по десяткам: на снимке — сплошь двускатные,
/// изредка ломаная мансарда и полувальма.
const HOUSE_FORMS: [GableForm; 10] = [
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gambrel,
    GableForm::HalfHip,
];
/// Сараи: в основном одним скатом.
const SHED_FORMS: [GableForm; 10] = [
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::LeanTo,
    GableForm::Gable,
    GableForm::Gable,
    GableForm::Gable,
];
/// Вальма остаётся крупному дому и изредка: из ста больших домов — трём.
const HIP_HOUSE_AREA_MIN: f32 = 120.0;
const HIP_SHARE_OF_100: u32 = 3;
/// Сколько двускатных из десяти несут слуховые окна, и с какого размера дома.
const DORMER_SHARE: u32 = 3;
const DORMER_HOUSE_MIN_LENGTH: f32 = 8.0;
const DORMER_HOUSE_MIN_WIDTH: f32 = 7.0;
/// Ломаной и полувальме нужна ширина, иначе излом и срез не читаются, м.
const GAMBREL_MIN_WIDTH: f32 = 5.5;
const HALF_HIP_MIN_WIDTH: f32 = 4.5;

/// Посев формы: тот же посев дома, но перемешанный. Его сырые разряды уже
/// разобраны на материал, цвет, яркость и высоту, и форма, взятая из них же,
/// ходила бы с ними в паре — все ломаные крыши оказались бы, скажем, синими.
fn shape_seed(seed: u32) -> u32 {
    seed.wrapping_mul(0x9E37_79B9).rotate_left(13) ^ 0x5bd1_e995
}

/// Какую крышу посев даёт прямоугольному дому.
pub(super) fn house_roof(building: &PolyArea, rect: &[Vec2; 4], seed: u32) -> HouseRoof {
    let seed = shape_seed(seed);
    let area = signed_ring_area(&building.outer).abs();
    let (length, width) = ((rect[1] - rect[0]).length(), (rect[2] - rect[1]).length());
    let shed = building.building_use != BuildingUse::House && area <= SHED_FOOTPRINT_MAX;
    let table = match shed {
        true => &SHED_FORMS,
        false => &HOUSE_FORMS,
    };
    let form = match table[((seed >> 5) % 10) as usize] {
        GableForm::Gambrel if width < GAMBREL_MIN_WIDTH => GableForm::Gable,
        GableForm::HalfHip if width < HALF_HIP_MIN_WIDTH => GableForm::Gable,
        form => form,
    };
    HouseRoof {
        form,
        hipped: !shed && area >= HIP_HOUSE_AREA_MIN && (seed >> 9) % 100 < HIP_SHARE_OF_100,
        dormers: form == GableForm::Gable
            && length >= DORMER_HOUSE_MIN_LENGTH
            && width >= DORMER_HOUSE_MIN_WIDTH
            && (seed >> 16) % 10 < DORMER_SHARE,
        seed,
    }
}

/// Крыша прямоугольного дома выбранной формы. `None` — дом не из скатной
/// когорты или контур не держит прямоугольник.
fn house_gable(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    choice: HouseRoof,
) -> Option<GableRoof> {
    if !is_pitched(building) {
        return None;
    }
    let rect = gable_rect(&building.outer)?;
    let frame = Frame::of(rect, lift);
    let rise = ridge_rise(frame.width);
    let mut roof = match choice.form {
        GableForm::HalfHip => half_hip(&frame, &ridge_lift, base, rise)
            .unwrap_or_else(|| plain_gable(&frame, &ridge_lift, base, rise)),
        GableForm::Gambrel => gambrel(&frame, &ridge_lift, base),
        GableForm::LeanTo => lean_to(&frame, &ridge_lift, base, (choice.seed >> 13) & 1 == 1),
        GableForm::Gable | GableForm::Cross => plain_gable(&frame, &ridge_lift, base, rise),
    };
    if choice.dormers && roof.form == GableForm::Gable {
        roof.dormers = dormers(&frame, &ridge_lift, base, rise, choice.seed);
    }
    Some(roof)
}

/// Слуховое окно: отступ передней стенки от карниза в долях полуширины,
/// высота стенки над скатом, крутизна его крыши, м.
const DORMER_SETBACK: f32 = 0.3;
const DORMER_FRONT: f32 = 0.9;
const DORMER_PITCH: f32 = 0.8;
/// Ширина окна — доля длины дома, зажатая в метрах.
const DORMER_WIDTH_SHARE: f32 = 0.22;
const DORMER_WIDTH: std::ops::RangeInclusive<f32> = 1.6..=2.6;

/// Слуховые окна на одном скате двускатной крыши: передняя стенка с окном,
/// щёчки и своя маленькая двускатная, упёртая коньком в скат. В плоских
/// режимах их нет — без подъёма это была бы заплата на скате.
fn dormers(
    frame: &Frame,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    rise: f32,
    seed: u32,
) -> Vec<DormerFace> {
    let [c0, _, _, c3] = frame.c;
    let half = frame.width / 2.0;
    if ridge_lift(1.0) == Vec2::ZERO || half <= 0.0 {
        return Vec::new();
    }
    let pitch = rise / half;
    let setback = DORMER_SETBACK * half;
    let (z0, z1) = (setback * pitch, setback * pitch + DORMER_FRONT);
    let wide =
        (frame.length * DORMER_WIDTH_SHARE).clamp(*DORMER_WIDTH.start(), *DORMER_WIDTH.end());
    let z_ridge = z1 + wide / 2.0 * DORMER_PITCH;
    if z_ridge > rise - 0.3 {
        return Vec::new();
    }
    // скат выбирает посев: вход в дом бывает с любой стороны
    let across = (c3 - c0).normalize_or_zero();
    let (origin, inward) = match (seed >> 14) & 1 {
        0 => (c0, across),
        _ => (c3, -across),
    };
    let at = |u: f32, v: f32, z: f32| origin + frame.long * u + inward * v + ridge_lift(z);
    let (eave, ridge) = (z1 / pitch, z_ridge / pitch);
    // окно одно: у одноэтажного дома мансарда одна, а ряд слуховых окон —
    // примета многоэтажного, по окну на подъезд. Не посередине, а где выпало
    let shift = ((seed >> 20) & 0xff) as f32 / 255.0;
    let centres = [frame.length * (0.35 + 0.3 * shift)];
    let front = -inward;
    let mut faces = Vec::new();
    for centre in centres {
        let (u0, u1) = (centre - wide / 2.0, centre + wide / 2.0);
        let (f0, f1) = (at(u0, setback, z0), at(u1, setback, z0));
        let (t0, t1) = (at(u0, setback, z1), at(u1, setback, z1));
        let apex = at(centre, setback, z_ridge);
        let (e0, e1) = (at(u0, eave, z1), at(u1, eave, z1));
        let back = at(centre, ridge, z_ridge);
        faces.push(DormerFace::Wall(vec![f0, e0, t0], -frame.long));
        faces.push(DormerFace::Wall(vec![f1, t1, e1], frame.long));
        faces.push(DormerFace::Wall(vec![f0, f1, t1, apex, t0], front));
        let pane = wide * 0.3;
        faces.push(DormerFace::Glass(
            vec![
                at(centre - pane, setback, z0 + 0.2),
                at(centre + pane, setback, z0 + 0.2),
                at(centre + pane, setback, z1 - 0.15),
                at(centre - pane, setback, z1 - 0.15),
            ],
            front,
        ));
        faces.push(DormerFace::Roof(
            vec![t0, apex, back, e0],
            slope_tone(base, -frame.long, 1.0),
        ));
        faces.push(DormerFace::Roof(
            vec![apex, t1, e1, back],
            slope_tone(base, frame.long, 1.0),
        ));
    }
    faces
}

/// Крестовая двускатная над домом из нескольких прямоугольников — буквой Г,
/// Т, П. Контур режется на прямоугольники тем же разрезом, что и гаражная
/// кровля ([`super::garages::split_rings`]); самый крупный кусок — корпус под
/// обычной двускатной вдоль своей длинной стороны, остальные — крылья, каждое
/// под своей двускатной, пристроенной к уже накрытому соседу:
///
/// * крыло у **длинной** стороны соседа идёт коньком поперёк неё, и конёк
///   уходит в скат соседа до ендовы;
/// * крыло у **торца** соседа продолжает его конёк, своим торцом упираясь в
///   его фронтон.
///
/// `None` — контур на прямоугольники не режется, кусок не пристроить ни к
/// кому или крыло шире соседа (его конёк встал бы выше соседского).
pub(super) fn cross_gable(
    building: &PolyArea,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    if !is_pitched(building) {
        return None;
    }
    let ring = merge_close_points(&building.outer, true, 0.2);
    // разрезов у Г два, у Т и П больше, и крылья пристраиваются не при
    // всяком: у Г, разрезанной «не той» хордой, крыло встаёт к торцу корпуса и
    // свешивается с него. Перебираются все разбиения, начиная с крупного корпуса
    let area = |rect: &[Vec2; 4]| (rect[1] - rect[0]).length() * (rect[2] - rect[1]).length();
    let mut candidates: Vec<(f32, Vec<[Vec2; 4]>)> = rect_splits(ring, CROSS_PIECES_MAX)
        .into_iter()
        .filter(|pieces| pieces.len() >= 2)
        .filter_map(|pieces| {
            let rects: Vec<[Vec2; 4]> = pieces
                .iter()
                .map(|piece| min_area_rect(piece))
                .collect::<Option<_>>()?;
            // кусок, недотягивающий до прямоугольника, накрылся бы крышей с
            // торчащим углом — ровно то, от чего двускатная отказывает контуру
            let rectangular = pieces
                .iter()
                .zip(&rects)
                .all(|(piece, rect)| signed_ring_area(piece).abs() >= RECT_FILL_MIN * area(rect));
            let largest = rects.iter().map(area).fold(0.0, f32::max);
            rectangular.then_some((largest, rects))
        })
        .collect();
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    candidates
        .into_iter()
        .find_map(|(_, rects)| cross_over(rects, lift, &ridge_lift, base))
}

/// Все разбиения кольца на куски не больше `budget` штук: из каждой вогнутой
/// вершины — оба разреза по её стенам ([`super::garages::cut_at`]), и дальше
/// рекурсивно. Почти прямоугольник не режется.
fn rect_splits(ring: Vec<Vec2>, budget: usize) -> Vec<Vec<Vec<Vec2>>> {
    let area = signed_ring_area(&ring);
    let fill = bounding_rect(&ring).map_or(0.0, |(_, fill)| fill);
    if budget <= 1 || ring.len() < 5 || fill >= SPLIT_FILL_DONE {
        return vec![vec![ring]];
    }
    let count = ring.len();
    let mut result = Vec::new();
    for at in 0..count {
        let (prev, here, next) = (
            ring[(at + count - 1) % count],
            ring[at],
            ring[(at + 1) % count],
        );
        let (back, ahead) = (here - prev, next - here);
        if back.perp_dot(ahead) * area.signum() >= 0.0 {
            continue;
        }
        for direction in [back, -ahead] {
            let Some(direction) = direction.try_normalize() else {
                continue;
            };
            let Some((near, far)) = super::garages::cut_at(&ring, at, direction) else {
                continue;
            };
            for left in rect_splits(near, budget - 1) {
                for right in rect_splits(far.clone(), budget - left.len()) {
                    if result.len() < SPLITS_MAX {
                        result.push(left.iter().cloned().chain(right).collect());
                    }
                }
            }
        }
    }
    match result.is_empty() {
        true => vec![vec![ring]],
        false => result,
    }
}

/// Заполнение, с которого кусок уже не режется, и потолок числа разбиений —
/// у Т и П они повторяются, а считать одно и то же дважды незачем.
const SPLIT_FILL_DONE: f32 = 0.97;
const SPLITS_MAX: usize = 24;

/// Крестовая двускатная над уже разрезанным контуром; `None` — какой-то
/// кусок не пристроить.
fn cross_over(
    mut rects: Vec<[Vec2; 4]>,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
) -> Option<GableRoof> {
    let area = |rect: &[Vec2; 4]| (rect[1] - rect[0]).length() * (rect[2] - rect[1]).length();
    let largest = (0..rects.len()).max_by(|a, b| area(&rects[*a]).total_cmp(&area(&rects[*b])))?;
    let main = Piece::of_rect(rects.swap_remove(largest));
    let main_rise = ridge_rise(main.width());
    let mut roof = plain_gable(&Frame::of(main.rect(), lift), &ridge_lift, base, main_rise);
    roof.form = GableForm::Cross;

    // крылья пристраиваются к уже накрытым кускам, пока пристраивается хоть одно
    let mut placed = vec![(main, main_rise)];
    while !rects.is_empty() {
        let mut attached = None;
        'search: for (index, rect) in rects.iter().enumerate() {
            for (parent, parent_rise) in &placed {
                if let Some(wing) = Wing::attach(parent, *parent_rise, rect) {
                    attached = Some((index, wing));
                    break 'search;
                }
            }
        }
        let (index, wing) = attached?;
        rects.swap_remove(index);
        placed.push(wing.push(&mut roof, lift, &ridge_lift, base));
    }
    Some(roof)
}

/// Больше кусков — это уже не дом с крыльями, а гребёнка, и вальма над ней
/// честнее пучка коньков.
const CROSS_PIECES_MAX: usize = 4;
/// Насколько стороны куска могут разойтись со сторонами соседа, м: обводка
/// частного дома неровная, и прямоугольники кусков стоят не идеально.
const CROSS_TOLERANCE: f32 = 0.6;

/// Прямоугольный кусок крыши в собственной системе: `d` — вдоль конька, `n` —
/// поперёк (CCW), пролёты `u` вдоль и `v` поперёк от `origin`.
#[derive(Clone, Copy)]
struct Piece {
    origin: Vec2,
    d: Vec2,
    n: Vec2,
    u: (f32, f32),
    v: (f32, f32),
}

impl Piece {
    /// Кусок по `min_area_rect`: конёк вдоль длинной стороны.
    fn of_rect(rect: [Vec2; 4]) -> Self {
        let d = (rect[1] - rect[0]).normalize_or_zero();
        Self {
            origin: rect[0],
            d,
            n: d.perp(),
            u: (0.0, (rect[1] - rect[0]).length()),
            v: (0.0, (rect[2] - rect[1]).length()),
        }
    }

    fn width(&self) -> f32 {
        self.v.1 - self.v.0
    }

    fn at(&self, u: f32, v: f32) -> Vec2 {
        self.origin + self.d * u + self.n * v
    }

    /// Прямоугольник CCW, `c0→c1` вдоль конька.
    fn rect(&self) -> [Vec2; 4] {
        [
            self.at(self.u.0, self.v.0),
            self.at(self.u.1, self.v.0),
            self.at(self.u.1, self.v.1),
            self.at(self.u.0, self.v.1),
        ]
    }

    /// Пролёты прямоугольника в системе этого куска — `None`, если его стороны
    /// не параллельны сторонам куска.
    fn project(&self, rect: &[Vec2; 4]) -> Option<((f32, f32), (f32, f32))> {
        let points: Vec<(f32, f32)> = rect
            .iter()
            .map(|p| {
                (
                    (*p - self.origin).dot(self.d),
                    (*p - self.origin).dot(self.n),
                )
            })
            .collect();
        let fold = |pick: fn(&(f32, f32)) -> f32| {
            points
                .iter()
                .map(pick)
                .fold((f32::MAX, f32::MIN), |(lo, hi), x| (lo.min(x), hi.max(x)))
        };
        let (u, v) = (fold(|p| p.0), fold(|p| p.1));
        // у повёрнутого прямоугольника углы не лягут на углы своей рамки
        let aligned = points.iter().all(|(pu, pv)| {
            ((pu - u.0).abs() < CROSS_TOLERANCE || (pu - u.1).abs() < CROSS_TOLERANCE)
                && ((pv - v.0).abs() < CROSS_TOLERANCE || (pv - v.1).abs() < CROSS_TOLERANCE)
        });
        aligned.then_some((u, v))
    }
}

/// Как крыло стоит к соседу.
enum Wing {
    /// У длинной стороны: `side` — знак стороны по `n` соседа.
    Side {
        parent: Piece,
        parent_rise: f32,
        side: f32,
        u: (f32, f32),
        far: f32,
    },
    /// У торца: крыло продолжает конёк соседа; `side` — знак торца по `d`.
    End {
        parent: Piece,
        side: f32,
        u: (f32, f32),
        v: (f32, f32),
    },
}

impl Wing {
    /// Пристраивается ли прямоугольник к соседу, и как.
    fn attach(parent: &Piece, parent_rise: f32, rect: &[Vec2; 4]) -> Option<Self> {
        let (u, v) = parent.project(rect)?;
        let t = CROSS_TOLERANCE;
        let width = parent.width();
        let within = |range: (f32, f32), of: (f32, f32)| range.0 >= of.0 - t && range.1 <= of.1 + t;
        if within(u, parent.u) && (u.1 - u.0) <= width * 1.05 {
            if (v.0 - parent.v.1).abs() < t {
                return Some(Self::Side {
                    parent: *parent,
                    parent_rise,
                    side: 1.0,
                    u,
                    far: v.1,
                });
            }
            if (v.1 - parent.v.0).abs() < t {
                return Some(Self::Side {
                    parent: *parent,
                    parent_rise,
                    side: -1.0,
                    u,
                    far: v.0,
                });
            }
        }
        if within(v, parent.v) {
            if (u.0 - parent.u.1).abs() < t {
                return Some(Self::End {
                    parent: *parent,
                    side: 1.0,
                    u: (parent.u.1, u.1),
                    v,
                });
            }
            if (u.1 - parent.u.0).abs() < t {
                return Some(Self::End {
                    parent: *parent,
                    side: -1.0,
                    u: (u.0, parent.u.0),
                    v,
                });
            }
        }
        None
    }

    /// Скаты и торец крыла — в крышу; наружу — кусок крыла и его подъём, к
    /// которым пристраиваются следующие.
    fn push(
        &self,
        roof: &mut GableRoof,
        lift: Vec2,
        ridge_lift: impl Fn(f32) -> Vec2,
        base: Srgba,
    ) -> (Piece, f32) {
        match *self {
            Self::Side {
                parent,
                parent_rise,
                side,
                u,
                far,
            } => {
                let line = match side > 0.0 {
                    true => parent.v.1,
                    false => parent.v.0,
                };
                let rise = ridge_rise(u.1 - u.0).min(parent_rise);
                // конёк крыла уходит в скат соседа, пока не упрётся в него: на
                // глубину своего подъёма, делённого на крутизну соседа
                let half = parent.width() / 2.0;
                let reach = (rise / (parent_rise / half).max(1e-3)).min(half);
                let at = |u: f32, v: f32| parent.at(u, v) + lift;
                let mid = (u.0 + u.1) / 2.0;
                let up = ridge_lift(rise);
                let (j0, j1) = (at(u.0, line), at(u.1, line));
                let (f0, f1) = (at(u.0, far), at(u.1, far));
                let far_ridge = at(mid, far) + up;
                let valley = at(mid, line - side * reach) + up;
                roof.slopes.push((
                    vec![j0, f0, far_ridge, valley],
                    slope_tone(base, -parent.d, 1.0),
                ));
                roof.slopes.push((
                    vec![f1, j1, valley, far_ridge],
                    slope_tone(base, parent.d, 1.0),
                ));
                // правый перпендикуляр ребра торца смотрит наружу, от соседа
                let (a, b) = match side > 0.0 {
                    true => (f1, f0),
                    false => (f0, f1),
                };
                roof.gables.push(((a, b), vec![a, b, far_ridge]));
                // сам кусок крыла: конёк от соседа наружу
                let d = parent.n * side;
                let origin = parent.at(u.0, line);
                let spread = (u.1 - u.0) * side;
                let piece = Piece {
                    origin,
                    d,
                    n: d.perp(),
                    u: (0.0, (far - line).abs()),
                    v: match spread > 0.0 {
                        true => (-spread, 0.0),
                        false => (0.0, -spread),
                    },
                };
                (piece, rise)
            }
            Self::End { parent, side, u, v } => {
                let piece = Piece { u, v, ..parent };
                let rise = ridge_rise(piece.width());
                let mut wing = plain_gable(&Frame::of(piece.rect(), lift), &ridge_lift, base, rise);
                // торец у соседа упирается в его стену: фронтона на нём нет
                let inner = match side > 0.0 {
                    true => 1,
                    false => 0,
                };
                wing.gables.remove(inner);
                roof.slopes.append(&mut wing.slopes);
                roof.gables.append(&mut wing.gables);
                (piece, rise)
            }
        }
    }
}
