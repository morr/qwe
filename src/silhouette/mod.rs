//! Силуэты пешек: процедурный атлас — диск человека, «уголёк» демона, ореол,
//! лужи крови и веера брызг ([`blood`]) и четыре позы лежащего тела
//! ([`figure`]), — посчитанный при старте по расстоянию до контура, плюс пол
//! размера в пикселях, ниже которого пешка на экране не ужимается.
//!
//! В `assets/` ни одного файла: художника у проекта нет, а форма, которую
//! видно с зума толпы, задаётся формулами. Все глифы лежат в **одном**
//! изображении, потому что спрайты батчатся по текстуре: люди, трупы и демоны
//! перемешаны по z, и две текстуры резали бы батч на каждом демоне.
//! Прозрачные тексели несут цвет кромки, а не чёрный — иначе линейная
//! фильтрация подмешивала бы к краю чёрную рамку.
//!
//! Мипы считаются здесь же (усреднение 2×2): точка в 2–4 px со 128-пиксельной
//! текстуры без них искрит при каждом шаге пешки.
//!
//! Посмотреть на атлас глазами:
//! `SILHOUETTE_DUMP=/tmp/atlas.png cargo test dump_atlas -- --ignored`.

use std::f32::consts::FRAC_PI_2;

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

pub mod blood;
pub mod figure;

/// Сторона ячейки атласа на нулевом мипе, px. 128, а не 64: ячейка трупа на
/// самом крупном зуме (0,05 м/px) — 75 логических px, на ретине 150
/// физических, и с 64 конечности фигуры расплывались бы.
const CELL_PX: u32 = 128;
/// Полуширина сглаживания кромки, px нулевого мипа.
const EDGE_PX: f32 = 1.0;
/// Радиус диска в долях полуячейки. Запас до края — на кромку и на глубокие
/// мипы, где сосед по атласу уже в двух текселях.
const DISC_RADIUS: f32 = 0.78;
/// Ширина тёмной каймы диска, в тех же долях: фишка с ободком, а не пятно.
const RIM_WIDTH: f32 = 0.16;
/// Яркость каймы — множитель к цвету спрайта: тот же цвет, темнее.
const RIM_SHADE: f32 = 0.45;
/// Уголёк: средний радиус, размах и число зубцов.
const EMBER_RADIUS: f32 = 0.60;
const EMBER_SPIKE: f32 = 0.20;
const EMBER_SPIKES: f32 = 7.0;
/// Яркость кончиков зубцов: ядро светится, края темнеют.
const EMBER_TIP_SHADE: f32 = 0.40;
/// Радиус ореола в долях полуячейки; за ним альфа ровно ноль.
const HALO_RADIUS: f32 = 0.98;

/// Ячейка атласа — форма, которую спрайт берёт по индексу.
///
/// Три формы стоят особняком, у остальных есть **номер варианта**: поз у тела
/// [`figure::POSES`], а луж и вееров брызг по десятку
/// ([`blood::POOLS`], [`blood::SPATTERS`]) — перечислять каждый вариант
/// отдельным именем значило бы держать три списка в согласии руками. Номер
/// ячейки в атласе считает [`Glyph::cell`], и семейства в нём идут подряд.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Glyph {
    /// Диск с тёмной каймой: человек.
    Disc,
    /// Семизубый уголёк с ярким ядром: демон.
    Ember,
    /// Радиальное затухание: свечение вокруг демона и всякое «сияние».
    Halo,
    /// Лужа крови под трупом — [`blood`]; номер брать через [`Glyph::pool`].
    Pool(usize),
    /// Веер брызг вокруг тела — [`blood`]; номер — через [`Glyph::spatter`].
    Spatter(usize),
    /// Лежащее тело — [`figure`]; номер позы — через [`Glyph::corpse`].
    Corpse(usize),
}

impl Glyph {
    /// Сколько ячеек в атласе.
    pub const COUNT: usize = 3 + blood::POOLS + blood::SPATTERS + figure::POSES;

    /// Номер ячейки в атласе. Семейства идут подряд — на этом стоит и
    /// раскладка `TextureAtlasLayout`, и порядок [`Glyph::all`].
    pub fn cell(self) -> usize {
        match self {
            Self::Disc => 0,
            Self::Ember => 1,
            Self::Halo => 2,
            Self::Pool(variant) => {
                debug_assert!(variant < blood::POOLS, "лужи №{variant} нет в атласе");
                3 + variant
            }
            Self::Spatter(variant) => {
                debug_assert!(variant < blood::SPATTERS, "брызг №{variant} нет в атласе");
                3 + blood::POOLS + variant
            }
            Self::Corpse(pose) => {
                debug_assert!(pose < figure::POSES, "позы №{pose} нет в атласе");
                3 + blood::POOLS + blood::SPATTERS + pose
            }
        }
    }

    /// Все ячейки атласа по порядку [`Glyph::cell`].
    pub fn all() -> impl Iterator<Item = Self> {
        [Self::Disc, Self::Ember, Self::Halo]
            .into_iter()
            .chain((0..blood::POOLS).map(Self::Pool))
            .chain((0..blood::SPATTERS).map(Self::Spatter))
            .chain((0..figure::POSES).map(Self::Corpse))
    }

    /// Поза лежащего тела номер `pose` (по модулю числа поз).
    pub fn corpse(pose: usize) -> Self {
        Self::Corpse(pose % figure::POSES)
    }

    /// Лужа крови номер `variant` (по модулю числа луж).
    pub fn pool(variant: usize) -> Self {
        Self::Pool(variant % blood::POOLS)
    }

    /// Веер брызг номер `variant` (по модулю числа вееров).
    pub fn spatter(variant: usize) -> Self {
        Self::Spatter(variant % blood::SPATTERS)
    }

    /// Номер позы, если глиф — лежащее тело.
    fn pose(self) -> Option<usize> {
        match self {
            Self::Corpse(pose) => Some(pose),
            _ => None,
        }
    }

    /// Куда под этот глиф ложится лужа, в координатах ячейки (−1…1): грудь
    /// фигуры; у остальных глифов — центр.
    pub fn pool_anchor(self) -> Vec2 {
        self.pose()
            .map_or(Vec2::ZERO, |pose| figure::Figure::pose(pose).pool_anchor())
    }
}

/// Переключает спрайт на другой глиф того же атласа; спрайт без атласа
/// (приложение без рендера) остаётся как был.
pub fn set_glyph(sprite: &mut Sprite, glyph: Glyph) {
    if let Some(atlas) = sprite.texture_atlas.as_mut() {
        atlas.index = glyph.cell();
    }
}

/// Атлас силуэтов. `None` — атлас не собран: приложение без рендера
/// (реплей, тесты) получает обычный квадратный `Sprite`, и это не ошибка.
#[derive(Resource, Default)]
pub struct Silhouettes(Option<Atlas>);

struct Atlas {
    image: Handle<Image>,
    layout: Handle<TextureAtlasLayout>,
}

impl Silhouettes {
    /// Спрайт с глифом `glyph`, покрашенный в `color`, размером `size` м.
    pub fn sprite(&self, glyph: Glyph, color: Color, size: Vec2) -> Sprite {
        let mut sprite = match &self.0 {
            Some(atlas) => Sprite::from_atlas_image(
                atlas.image.clone(),
                TextureAtlas {
                    layout: atlas.layout.clone(),
                    index: glyph.cell(),
                },
            ),
            None => Sprite::default(),
        };
        sprite.color = color;
        sprite.custom_size = Some(size);
        sprite
    }
}

/// Как пешка стоит относительно камеры: тело в метрах и пол в логических
/// пикселях, ниже которого её спрайт не ужимается. На дальнем зуме метровый
/// человек — 0,2 px и исчезает, а толпа обязана читаться с любого расстояния;
/// пол превращает её в зерно, демона — в заметную точку.
///
/// `Sprite::custom_size` при этом принадлежит этому модулю: спавн задаёт тело,
/// [`size_fresh_silhouettes`] и [`resize_silhouettes_on_zoom`] пишут размер.
#[derive(Component, Reflect, Clone, Copy, Debug, PartialEq)]
#[reflect(Component)]
pub struct Silhouette {
    /// Тело в метрах, как оно рисуется при 1 м = 1 px и крупнее.
    pub body: Vec2,
    /// Пол на экране, логических px по меньшей стороне тела.
    pub min_px: f32,
}

impl Silhouette {
    pub const fn new(body: Vec2, min_px: f32) -> Self {
        Self { body, min_px }
    }

    /// Размер спрайта при зуме `zoom` (метров в логическом пикселе): тело,
    /// растянутое до пола, если тело на экране меньше пола.
    pub fn drawn_size(&self, zoom: f32) -> Vec2 {
        let floor = zoom * self.min_px;
        let scale = (floor / self.body.min_element()).max(1.0);
        self.body * scale
    }
}

pub struct SilhouettePlugin;

impl Plugin for SilhouettePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Silhouettes>()
            .register_type::<Silhouette>()
            .add_systems(Startup, build_atlas)
            .add_systems(
                Update,
                (size_fresh_silhouettes, resize_silhouettes_on_zoom).chain(),
            );
    }
}

/// Собирает атлас — один раз на процесс: глифы не зависят ни от города, ни от
/// прогона, а `Handle`-ы переживают любую смену мира.
fn build_atlas(
    mut images: ResMut<Assets<Image>>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
    mut silhouettes: ResMut<Silhouettes>,
) {
    let started = std::time::Instant::now();
    let atlas = atlas_image();
    info!(
        "silhouette atlas: {}×{} px, {} mips, {} glyphs in {:.1?}",
        atlas.width(),
        atlas.height(),
        atlas.texture_descriptor.mip_level_count,
        Glyph::COUNT,
        started.elapsed()
    );
    let image = images.add(atlas);
    let layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(CELL_PX),
        Glyph::COUNT as u32,
        1,
        None,
        None,
    ));
    silhouettes.0 = Some(Atlas { image, layout });
}

/// Изображение атласа со всей цепочкой мипов.
fn atlas_image() -> Image {
    let width = CELL_PX * Glyph::COUNT as u32;
    let height = CELL_PX;
    let level0 = rasterize_atlas(width, height);

    let mut data = level0.clone();
    let mut levels = 1;
    let (mut w, mut h, mut level) = (width, height, level0.clone());
    while w > 1 || h > 1 {
        let (w2, h2, next) = downsample(w, h, &level);
        data.extend_from_slice(&next);
        levels += 1;
        (w, h, level) = (w2, h2, next);
    }

    let mut image = Image::new(
        Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level0,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = levels;
    image.data = Some(data);
    // приложение целиком на `ImagePlugin::default_nearest()`, но силуэт —
    // гладкая фигура, а не пиксель-арт: кромка сглажена и должна остаться
    // такой на любом зуме
    image.sampler = ImageSampler::linear();
    image
}

/// Нулевой мип: все ячейки подряд, RGBA8, строка 0 — верх. Обход по ячейкам,
/// а не по строкам: фигура трупа собирается один раз на ячейку, не на тексель.
fn rasterize_atlas(width: u32, height: u32) -> Vec<u8> {
    let mut data = vec![0u8; (width * height * 4) as usize];
    let byte = |channel: f32| (channel.clamp(0.0, 1.0) * 255.0).round() as u8;
    for (cell, glyph) in Glyph::all().enumerate() {
        let shape = Shape::of(glyph);
        for row in 0..CELL_PX {
            for column in 0..CELL_PX {
                let centre = Vec2::new(column as f32 + 0.5, (CELL_PX - 1 - row) as f32 + 0.5);
                let p = centre / CELL_PX as f32 * 2.0 - 1.0;
                let (shade, alpha) = shape.texel(p, edge());
                let x = cell as u32 * CELL_PX + column;
                let i = ((row * width + x) * 4) as usize;
                data[i..i + 4].copy_from_slice(&[
                    byte(shade),
                    byte(shade),
                    byte(shade),
                    byte(alpha),
                ]);
            }
        }
    }
    data
}

/// Полуширина сглаживания кромки в единицах ячейки (−1…1).
fn edge() -> f32 {
    EDGE_PX * 2.0 / CELL_PX as f32
}

/// Форма глифа, собранная **один раз на ячейку**. У лежащего тела это два
/// десятка капсул скелета, у брызг — три десятка капель: собирать их на
/// каждый тексель значило бы умножить работу атласа на площадь ячейки.
enum Shape {
    /// Считается прямо из точки, собирать нечего: диск, уголёк, ореол.
    Plain(Glyph),
    Body(figure::Figure),
    Blood(blood::Stain),
}

impl Shape {
    fn of(glyph: Glyph) -> Self {
        match glyph {
            Glyph::Corpse(pose) => Self::Body(figure::Figure::pose(pose)),
            Glyph::Pool(variant) => Self::Blood(blood::Stain::pool(variant)),
            Glyph::Spatter(variant) => Self::Blood(blood::Stain::spatter(variant)),
            plain => Self::Plain(plain),
        }
    }

    /// Тексель глифа в точке `p` ячейки (−1…1 по обеим осям): яркость
    /// (множитель к цвету спрайта) и альфа.
    fn texel(&self, p: Vec2, edge: f32) -> (f32, f32) {
        match self {
            Self::Plain(glyph) => plain_texel(*glyph, p, edge),
            Self::Body(figure) => figure.texel(p, edge),
            Self::Blood(stain) => stain.texel(p, edge),
        }
    }
}

/// Тексель глифа, у которого нет собираемой формы: одна формула от точки.
fn plain_texel(glyph: Glyph, p: Vec2, edge: f32) -> (f32, f32) {
    let r = p.length();
    match glyph {
        Glyph::Disc => rimmed(DISC_RADIUS - r, edge, RIM_WIDTH, RIM_SHADE),
        Glyph::Ember => {
            // зубец смотрит вверх: фаза сдвинута на четверть оборота
            let theta = p.y.atan2(p.x) - FRAC_PI_2;
            let radius = EMBER_RADIUS + EMBER_SPIKE * (EMBER_SPIKES * theta).cos();
            let inside = radius - r;
            let alpha = smoothstep(-edge, edge, inside);
            let shade = 1.0 - (1.0 - EMBER_TIP_SHADE) * smoothstep(0.0, radius, r);
            (shade, alpha)
        }
        Glyph::Halo => {
            let falloff = (1.0 - r / HALO_RADIUS).clamp(0.0, 1.0);
            (1.0, falloff * falloff)
        }
        Glyph::Pool(_) | Glyph::Spatter(_) | Glyph::Corpse(_) => {
            unreachable!("тела и кровь собраны в Shape::of")
        }
    }
}

/// Капсула с двумя радиусами — оболочка двух кругов; `a == b` даёт круг.
/// Общий примитив рисунка: из неё сложены и кости лежащего тела
/// ([`figure`]), и доли лужи с каплями брызг ([`blood`]).
struct Capsule {
    a: Vec2,
    b: Vec2,
    ra: f32,
    rb: f32,
}

impl Capsule {
    fn new(a: Vec2, b: Vec2, ra: f32, rb: f32) -> Self {
        Self { a, b, ra, rb }
    }

    fn dot(centre: Vec2, radius: f32) -> Self {
        Self::new(centre, centre, radius, radius)
    }

    /// Габаритный круг: центр и радиус. Тем, кто складывает пятно из десятков
    /// капсул, он экономит вызов [`Capsule::distance`] на далёкой точке.
    fn bound(&self) -> (Vec2, f32) {
        (
            (self.a + self.b) / 2.0,
            (self.b - self.a).length() / 2.0 + self.ra.max(self.rb),
        )
    }

    /// Расстояние со знаком до контура (внутри — отрицательное).
    fn distance(&self, p: Vec2) -> f32 {
        let ab = self.b - self.a;
        let h = ab.length();
        if h < 1e-4 {
            return (p - self.a).length() - self.ra.max(self.rb);
        }
        let along = ab / h;
        let q = p - self.a;
        // в системе капсулы: y вдоль оси, x поперёк, по модулю
        let q = Vec2::new(q.perp_dot(along).abs(), q.dot(along));
        let slope = (self.ra - self.rb) / h;
        let cos = (1.0 - slope * slope).max(0.0).sqrt();
        let k = q.dot(Vec2::new(-slope, cos));
        if k < 0.0 {
            q.length() - self.ra
        } else if k > cos * h {
            (q - Vec2::new(0.0, h)).length() - self.rb
        } else {
            q.dot(Vec2::new(cos, slope)) - self.ra
        }
    }
}

fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    let t = ((x - from) / (to - from)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Тело с тёмной каймой — форма диска и форма фигуры: альфа по контуру,
/// яркость `rim_shade` у самого края и полная глубже каймы шириной `rim`.
/// `inside` — расстояние внутрь от контура; `inside`, `rim` и `edge` — в одних
/// единицах, каких именно, решает вызывающий (диск считает в долях полуячейки,
/// фигура — в ростах).
fn rimmed(inside: f32, edge: f32, rim: f32, rim_shade: f32) -> (f32, f32) {
    let alpha = smoothstep(-edge, edge, inside);
    let shade = rim_shade + (1.0 - rim_shade) * smoothstep(rim - edge, rim + edge, inside);
    (shade, alpha)
}

/// Следующий мип: среднее по блоку 2×2 (2×1 / 1×2 у вырожденной стороны).
fn downsample(width: u32, height: u32, data: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w2, h2) = ((width / 2).max(1), (height / 2).max(1));
    let mut out = Vec::with_capacity((w2 * h2 * 4) as usize);
    for y in 0..h2 {
        for x in 0..w2 {
            let mut sum = [0u32; 4];
            let mut taps = 0;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = (x * 2 + dx).min(width - 1);
                    let sy = (y * 2 + dy).min(height - 1);
                    let i = ((sy * width + sx) * 4) as usize;
                    for (channel, acc) in sum.iter_mut().enumerate() {
                        *acc += u32::from(data[i + channel]);
                    }
                    taps += 1;
                }
            }
            out.extend(sum.iter().map(|acc| (acc / taps) as u8));
        }
    }
    (w2, h2, out)
}

/// Зум камеры — метров в логическом пикселе; `None` без камеры (реплей).
fn camera_zoom(camera: Option<Single<&Transform, With<Camera2d>>>) -> Option<f32> {
    camera.map(|camera| camera.scale.x)
}

/// Свежий или переписанный силуэт (спавн, труп) получает размер под текущий
/// зум — иначе пешка, родившаяся на дальнем плане, ждала бы первого щелчка
/// колеса.
pub fn size_fresh_silhouettes(
    camera: Option<Single<&Transform, With<Camera2d>>>,
    mut fresh: Query<(&Silhouette, &mut Sprite), Changed<Silhouette>>,
) {
    let Some(zoom) = camera_zoom(camera) else {
        return;
    };
    for (silhouette, mut sprite) in &mut fresh {
        sprite.custom_size = Some(silhouette.drawn_size(zoom));
    }
}

/// Полный проход по всем силуэтам — только на смену зума: тысячи записей в
/// `Sprite` за щелчок колеса, а не за кадр.
pub fn resize_silhouettes_on_zoom(
    camera: Option<Single<&Transform, With<Camera2d>>>,
    mut last_zoom: Local<f32>,
    mut all: Query<(&Silhouette, &mut Sprite)>,
) {
    let Some(zoom) = camera_zoom(camera) else {
        return;
    };
    if zoom == *last_zoom {
        return;
    }
    *last_zoom = zoom;
    for (silhouette, mut sprite) in &mut all {
        sprite.custom_size = Some(silhouette.drawn_size(zoom));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_is_drawn_as_is_while_it_is_bigger_than_the_floor() {
        let silhouette = Silhouette::new(Vec2::splat(1.0), 2.0);
        // 0,4 м/px: метр — это 2,5 px, пол в 2 px не достигнут
        assert_eq!(silhouette.drawn_size(0.4), Vec2::splat(1.0));
    }

    #[test]
    fn body_is_stretched_to_the_floor_when_zoomed_out() {
        let silhouette = Silhouette::new(Vec2::new(1.6, 0.8), 2.0);
        // 4,5 м/px: пол — 9 м по меньшей стороне, пропорции сохранены
        assert_eq!(silhouette.drawn_size(4.5), Vec2::new(18.0, 9.0));
    }

    /// Форма глифа на каждый вызов — путь тестов; атлас собирает её один раз
    /// на ячейку ([`Shape::of`]).
    fn texel(glyph: Glyph, p: Vec2) -> (f32, f32) {
        Shape::of(glyph).texel(p, edge())
    }

    #[test]
    fn every_glyph_is_transparent_past_the_cell_corner() {
        for glyph in Glyph::all() {
            let (_, outside) = texel(glyph, Vec2::new(0.99, 0.99));
            assert_eq!(outside, 0.0, "{glyph:?} leaks past the cell corner");
        }
    }

    /// Плотное там, где должно быть плотным: тело — под своей лужей, лужа — в
    /// середине. Брызг тут нет намеренно: у веера середина пустая (там стоит
    /// лужа своим спрайтом), и его проверяет [`blood`].
    #[test]
    fn bodies_and_pools_are_solid_at_their_anchor() {
        let bodies = (0..figure::POSES).map(Glyph::corpse);
        for glyph in bodies.chain((0..blood::POOLS).map(Glyph::pool)) {
            let (_, anchor) = texel(glyph, glyph.pool_anchor());
            assert_eq!(anchor, 1.0, "{glyph:?} is not solid at its anchor");
        }
        for glyph in [Glyph::Disc, Glyph::Ember, Glyph::Halo] {
            let (shade, _) = texel(glyph, Vec2::ZERO);
            assert_eq!(shade, 1.0, "{glyph:?} is not brightest at the centre");
        }
    }

    /// Номер варианта берётся по модулю: `Glyph::pool(POOLS)` — снова нулевая.
    #[test]
    fn a_variant_number_wraps_around_its_family() {
        assert_eq!(Glyph::corpse(figure::POSES), Glyph::Corpse(0));
        assert_eq!(Glyph::pool(blood::POOLS + 1), Glyph::Pool(1));
        assert_eq!(Glyph::spatter(blood::SPATTERS), Glyph::Spatter(0));
        assert_eq!(Glyph::Corpse(2).pose(), Some(2));
        assert_eq!(Glyph::Disc.pose(), None);
        assert_eq!(Glyph::Disc.pool_anchor(), Vec2::ZERO);
    }

    /// Ячейка глифа в атласе — его [`Glyph::cell`]: [`rasterize_atlas`] рисует
    /// ячейки по порядку [`Glyph::all`], а [`Silhouettes::sprite`] берёт
    /// индекс из `cell`. Разъедутся эти два порядка — и каждая пешка возьмёт
    /// чужую картинку, молча и на всех зумах сразу.
    #[test]
    fn every_glyph_is_rasterised_into_the_cell_its_index_names() {
        let mut count = 0;
        for (cell, glyph) in Glyph::all().enumerate() {
            assert_eq!(glyph.cell(), cell, "{glyph:?} is not in cell {cell}");
            count += 1;
        }
        assert_eq!(count, Glyph::COUNT, "атлас режется не на столько ячеек");
    }

    #[test]
    fn set_glyph_moves_the_atlas_index_and_leaves_a_plain_sprite_alone() {
        let mut plain = Sprite::default();
        set_glyph(&mut plain, Glyph::corpse(1));
        assert!(plain.texture_atlas.is_none());

        let mut atlas = Sprite::from_atlas_image(
            Handle::default(),
            TextureAtlas {
                layout: Handle::default(),
                index: Glyph::Disc.cell(),
            },
        );
        set_glyph(&mut atlas, Glyph::corpse(1));
        assert_eq!(atlas.texture_atlas.unwrap().index, Glyph::corpse(1).cell());
    }

    /// Не проверка, а инструмент: пишет нулевой мип атласа в PNG, чтобы
    /// посмотреть на глифы глазами. `SILHOUETTE_DUMP=<путь> cargo test
    /// dump_atlas -- --ignored`; прозрачное лучше смотреть на сером фоне
    /// (`magick atlas.png -background gray50 -flatten out.png`).
    #[test]
    #[ignore = "пишет файл; запускать руками с SILHOUETTE_DUMP=<путь>"]
    fn dump_atlas() {
        let path = std::env::var("SILHOUETTE_DUMP").expect("SILHOUETTE_DUMP=<путь к png>");
        // кодировщику нужен ровно нулевой мип: цепочку отрезаем
        let mut image = atlas_image();
        let level0 = (image.width() * image.height() * 4) as usize;
        image.data.as_mut().expect("данные атласа").truncate(level0);
        image.texture_descriptor.mip_level_count = 1;
        image
            .try_into_dynamic()
            .expect("атлас — RGBA8")
            .save(&path)
            .expect("png не записался");
        println!("atlas written to {path}");
    }

    #[test]
    fn capsule_distance_is_negative_inside_and_positive_outside() {
        let capsule = Capsule::new(Vec2::ZERO, Vec2::X, 0.2, 0.1);
        assert!(capsule.distance(Vec2::new(0.5, 0.0)) < 0.0);
        assert!(capsule.distance(Vec2::new(0.5, 0.5)) > 0.0);
        // концы — окружности своих радиусов
        assert!((capsule.distance(Vec2::new(-0.2, 0.0))).abs() < 1e-5);
        assert!((capsule.distance(Vec2::new(1.1, 0.0))).abs() < 1e-5);
        // габарит накрывает оба конца целиком
        let (centre, bound) = capsule.bound();
        assert_eq!(centre, Vec2::new(0.5, 0.0));
        assert!((bound - 0.7).abs() < 1e-5);
    }

    #[test]
    fn disc_rim_is_darker_than_its_core() {
        let (rim, alpha) = texel(Glyph::Disc, Vec2::new(DISC_RADIUS - RIM_WIDTH / 2.0, 0.0));
        assert_eq!(alpha, 1.0);
        assert!(rim < 0.5, "rim shade {rim} should be near RIM_SHADE");
    }

    #[test]
    fn mip_chain_ends_in_one_texel_and_keeps_the_layout() {
        let image = atlas_image();
        let width = CELL_PX * Glyph::COUNT as u32;
        // цепочка идёт до 1×1, и её длина считается здесь же: число ячеек
        // меняется вместе с числом вариантов крови, а вот «до одного текселя»
        // — свойство самой цепочки
        let mut levels = 1;
        let mut expected = 0;
        let (mut w, mut h) = (width, CELL_PX);
        loop {
            expected += w * h * 4;
            if w == 1 && h == 1 {
                break;
            }
            levels += 1;
            (w, h) = ((w / 2).max(1), (h / 2).max(1));
        }
        assert_eq!(image.texture_descriptor.mip_level_count, levels);
        assert_eq!(image.data.as_ref().map(Vec::len), Some(expected as usize));
    }

    #[test]
    fn downsample_averages_the_block() {
        // 2×2 → 1×1: среднее четырёх текселей
        let data = [
            0, 0, 0, 0, 100, 100, 100, 100, 200, 200, 200, 200, 100, 100, 100, 100,
        ];
        let (w, h, out) = downsample(2, 2, &data);
        assert_eq!((w, h), (1, 1));
        assert_eq!(out, vec![100, 100, 100, 100]);
    }
}
