//! Скатные крыши малых домов. `roof:shape` в OSM стоит у 283 зданий Тулы из
//! 7465, так что форма крыши не читается из тегов, а **выводится**: скатная
//! когорта — частный сектор классификатора ([`super::classify`]), дом
//! (`BuildingUse::House`) и мелкая низкая коробка без назначения; остальное —
//! плоское. Плоская крыша у частного сектора была
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

use super::classify::BuildingClass;
use super::{HouseFootprint, shade_by_light};
use crate::map::meshing::{merge_close_points, miter_offsets, ring_perimeter};
use crate::map::osm::model::{distance_to_segment, signed_ring_area};
use crate::map::osm::{AreaKind, BuildingUse, PolyArea};

mod gable;

pub(in crate::map::buildings) use gable::gable_roof;
use gable::{HouseRoof, cross_gable, gable_over, gable_rect, house_gable, house_roof};

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
    let (rect, rect_fill) = bounding_rect(building, None)?;
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
#[allow(clippy::too_many_arguments)]
pub(super) fn roofing_of(
    shape: RoofShape,
    building: &PolyArea,
    class: &BuildingClass,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    seed: u32,
    fp: Option<HouseFootprint>,
) -> Roofing {
    match shape {
        RoofShape::Auto => roofing(building, class, lift, ridge_lift, base, seed, fp),
        RoofShape::Flat => Roofing::Flat,
        RoofShape::Gable => gable_roof(building, class, lift, &ridge_lift, base, fp)
            .map_or(Roofing::Flat, Roofing::Gable),
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
            house_gable(building, class, lift, &ridge_lift, base, choice, fp)
                .map_or(Roofing::Flat, Roofing::Gable)
        }
        RoofShape::Cross => cross_gable(building, class, lift, &ridge_lift, base)
            .map_or(Roofing::Flat, Roofing::Gable),
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
///
/// Скатная когорта — классификатор ([`super::classify`]): частный сектор без
/// двора. Кто в неё попал, решается один раз на дом, вместе с классом.
pub(super) fn roofing(
    building: &PolyArea,
    class: &BuildingClass,
    lift: Vec2,
    ridge_lift: impl Fn(f32) -> Vec2,
    base: Srgba,
    seed: u32,
    fp: Option<HouseFootprint>,
) -> Roofing {
    if let Some(form) = landmark_roof(building, fp) {
        return landmark_roofing(form, building, lift, ridge_lift, base, fp);
    }
    if !class.is_pitched() {
        return Roofing::Flat;
    }
    if let Some(rect) = gable_rect(building, fp) {
        let choice = house_roof(building, &rect, seed);
        // редкая вальма — только большому дому, и только если посев так решил
        if choice.hipped
            && let Some(hip) = hip_roof(building, lift, &ridge_lift, base)
        {
            return Roofing::Hip(hip);
        }
        if let Some(roof) = house_gable(building, class, lift, &ridge_lift, base, choice, fp) {
            return Roofing::Gable(roof);
        }
    }
    if let Some(roof) = cross_gable(building, class, lift, &ridge_lift, base) {
        return Roofing::Gable(roof);
    }
    match hip_roof(building, lift, &ridge_lift, base) {
        Some(roof) => Roofing::Hip(roof),
        // на совсем узком контуре вальма выворачивается, а двускатная не
        // встала — пусть будет плоской, это по крайней мере не врёт
        None => gable_roof(building, class, lift, &ridge_lift, base, fp)
            .map_or(Roofing::Flat, Roofing::Gable),
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
    pub(super) fn add(&mut self, class: &BuildingClass, drawn: RoofShape) {
        if !class.is_pitched() || class.is_landmark() {
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
pub(super) fn landmark_roof(
    building: &PolyArea,
    fp: Option<HouseFootprint>,
) -> Option<LandmarkRoof> {
    if building.kind == AreaKind::Kremlin {
        return Some(super::fortress::roof_form(building));
    }
    match building.building_use {
        BuildingUse::Church(sacred) => Some(super::temples::roof_form(sacred, building, fp)),
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
    fp: Option<HouseFootprint>,
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
                fp,
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
pub(super) fn landmark_rise(building: &PolyArea, fp: Option<HouseFootprint>) -> f32 {
    let hip = || hip_plan(&building.outer).map_or(0.0, |(_, inset)| hip_rise(inset));
    match landmark_roof(building, fp) {
        None | Some(LandmarkRoof::Flat) => 0.0,
        Some(LandmarkRoof::Hip) => hip(),
        Some(LandmarkRoof::SteepGable) => match gable_rect(building, fp) {
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

/// Насколько площадка назначенной крыши вдвинута от карниза, м, — спутник
/// [`landmark_rise`]: та отвечает, на какой высоте стоит глава, эта — насколько
/// от края контура. У вальмы это вылет её ската: барабан, поставленный вплотную
/// к карнизу, стоял бы на скате и свешивался бы с кровли (Тула, way 234273451 —
/// две восточные главы наезжали на скат). У плоской крыши вдвига нет.
///
/// Шатёр и крутая двускатная площадки не дают вовсе, и ноль здесь — не «можно
/// везде»: на них главы и не раскладываются (шатёр несёт башня, крутую
/// двускатную — западный храм, а у него главы нет).
pub(super) fn landmark_inset(building: &PolyArea, fp: Option<HouseFootprint>) -> f32 {
    let hip = || hip_plan(&building.outer).map_or(0.0, |(_, inset)| inset);
    match landmark_roof(building, fp) {
        Some(LandmarkRoof::Hip) => hip(),
        Some(LandmarkRoof::SteepGable) if gable_rect(building, fp).is_none() => hip(),
        _ => 0.0,
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
    let inset =
        HIP_INSET.min(HIP_INSET_SHARE * signed_ring_area(&ring).abs() / ring_perimeter(&ring));
    (inset >= MIN_HIP_INSET).then_some((ring, inset))
}

/// Подъём конька вальмы над карнизом: тот же скат, что у двускатной, только
/// разложенный на вылет каймы, а не на половину ширины дома.
fn hip_rise(inset: f32) -> f32 {
    (inset * ROOF_PITCH).min(ROOF_RISE_MAX)
}

// Скатная когорта — классификатор ([`super::classify::BuildingClass::is_pitched`]):
// частный сектор без двора. Раньше предикат жил здесь, дублировал развилку
// классификатора и разошёлся с ней — у скатной когорты гварда этажности не
// было, и девятиэтажка на мелкой коробке получала черепичную двускатную
// поверх панельных стен.

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
/// которому игра отказывает двускатной, а не пересчитанное своё. Пятно дома
/// приходит готовым ([`HouseFootprint`]); `None` — пятна нет, оно считается
/// на месте, как раньше.
fn bounding_rect(building: &PolyArea, fp: Option<HouseFootprint>) -> Option<([Vec2; 4], f32)> {
    let rect = fp.or_else(|| HouseFootprint::of(building))?.rect();
    let area = rect_area(&rect);
    if area <= 0.0 {
        return None;
    }
    Some((rect, signed_ring_area(&building.outer).abs() / area))
}

/// Площадь прямоугольника, заданного углами по обходу.
fn rect_area(rect: &[Vec2; 4]) -> f32 {
    (rect[1] - rect[0]).length() * (rect[2] - rect[1]).length()
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

/// Пятно, до которого дом без назначения считается сараем, баней или летней
/// кухней, м²: такой кроется одним скатом и стоит в один низкий этаж.
pub(super) const SHED_FOOTPRINT_MAX: f32 = 40.0;

/// Посев формы: тот же посев дома, но перемешанный. Его сырые разряды уже
/// разобраны на материал, цвет, яркость и высоту, и форма, взятая из них же,
/// ходила бы с ними в паре — все ломаные крыши оказались бы, скажем, синими.
fn shape_seed(seed: u32) -> u32 {
    seed.wrapping_mul(0x9E37_79B9).rotate_left(13) ^ 0x5bd1_e995
}
