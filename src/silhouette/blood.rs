//! Кровь под телом: лужа и брызги — две разные вещи, и потому два глифа.
//!
//! **Лужа натекает.** Она растёт из-под тела наружу, у неё густая тёмная
//! середина и тонкая светлая кромка, а форма — не круг, а несколько слипшихся
//! долей, из которых по уклону ушёл ручеёк. На экране она и вправду
//! растекается: размер спрайта ведёт `human::look::spread_blood`.
//!
//! **Брызги ложатся разом**, в момент удара, и потому не растут вовсе: капли
//! летят веером от тела, дальние мельче ближних и вытянуты по полёту
//! (у капли, упавшей под углом, узкий хвост смотрит вперёд), а на обратной
//! стороне остаётся редкий отброс. Ячейка брызг заметно шире ячейки лужи —
//! отсюда два спрайта под трупом, а не один: одному пришлось бы выбирать
//! между разрешением лужи и размахом веера.
//!
//! Вариантов [`POOLS`] и [`SPATTERS`]; форма варианта разыграна из его
//! номера (splitmix64) — тем же приёмом, каким крыша здания разыграна из своей
//! первой вершины. Набор поэтому одинаков в любом запуске, хранить его не
//! нужно, а тел на карте тысячи, и повтор виден: [`super::Glyph::pool`] и
//! [`super::Glyph::spatter`] раздают варианты по битам сущности
//! (`human::look::blood_look`), поверх ложатся свой поворот, размер и тон.
//!
//! Число вариантов **ничего не стоит в кадре**: ячейки считаются один раз при
//! старте, вариант выбирается один раз при спавне трупа, а спрайты и дальше
//! батчатся одной текстурой — платит только сборка атласа и его размер
//! (ячейка 128 px, `CELL_PX`).
//!
//! Толщина слоя решает и прозрачность, и цвет. `inside` — насколько глубоко
//! точка сидит внутри контура — переводится в «глубину» пятна, а из неё в
//! альфу (плёнка у кромки едва видна) и в яркость ([`CORE_SHADE`]: где крови
//! много, там она почти чёрная). Формула одна на лужу и на каплю, и мелкая
//! капля выходит тонкой просто потому, что мелкая, — как оно и бывает.

use std::f32::consts::TAU;

use bevy::prelude::*;

use super::{Capsule, smoothstep};
use crate::rng::splitmix64;

/// Сколько луж лежит в атласе.
pub const POOLS: usize = 10;
/// Сколько раскладок брызг лежит в атласе.
pub const SPATTERS: usize = 10;

/// Глубина, на которой пятно становится непрозрачным, в долях полуячейки:
/// ближе к кромке — плёнка, глубже — лужа. Набирается **по корню**, то есть
/// круто у самой кромки и вяло дальше: иначе капля в три сотых ячейки не
/// добирает и половины глубины и выходит розовой, а капля крови на мостовой
/// розовой не бывает — тонкая она только по краю.
const DEPTH: f32 = 0.032;
/// Альфа плёнки у самой кромки, доля от полной. Ниже — и на светлой мостовой
/// (её линейная яркость 0,74 против 0,3 у крови) сквозь плёнку проступает
/// земля, а пятно уходит в блёклую бежевую тень.
const FILM: f32 = 0.70;
/// Яркость густой середины — множитель к цвету спрайта. Цвет спрайта задаёт
/// **тонкую** кровь: ярче него глиф покрасить не может (яркость пишется в
/// 8 бит и обрезается единицей), поэтому светлое — плёнка, тёмное — глубина.
///
/// Значение читается как sRGB (текстура атласа `Rgba8UnormSrgb`), то есть в
/// линейном свете 0,55 — это ≈0,26: середина лужи темнее кромки вчетверо, а
/// не вдвое. Считать здесь линейными долями — верный способ получить чёрное.
const CORE_SHADE: f32 = 0.55;
/// Насколько дальше габарита капсулы её ещё стоит считать: гармоники кромки
/// плюс запас на сглаживание. Ближе — вклад точно нулевой, и капсулу можно
/// не трогать; на брызгах это снимает почти всю работу.
const CULL_SLACK: f32 = 0.06;

// --- Лужа ---
/// Радиус центральной доли, в долях полуячейки. Нарочно меньше долей вокруг:
/// когда середина крупнее их всех, объединение выходит кругом, и десять
/// вариантов читаются одним.
const POOL_CORE: (f32, f32) = (0.24, 0.32);
/// Сколько долей налипает вокруг центральной.
const POOL_LOBES: (usize, usize) = (5, 8);
/// Как далеко от середины сидит доля и какого она радиуса.
const POOL_LOBE_DIST: (f32, f32) = (0.18, 0.36);
const POOL_LOBE_RADIUS: (f32, f32) = (0.16, 0.32);
/// Вытянутость лужи: доли ставятся не по кругу, а по эллипсу со своим
/// наклоном. Кровь растекается по уклону, и ровно круглое пятно — это клякса,
/// а не лужа; на глаз вытянутость различает варианты сильнее всего прочего.
const POOL_STRETCH: (f32, f32) = (1.0, 1.35);
/// Мягкость слипания долей: доли не пересекаются кругами, а сливаются в одно
/// пятно с перешейками — этим лужа и отличается от кучки блинов.
const POOL_WELD: f32 = 0.14;
/// Гармоники кромки: порядок и амплитуда. Мелкая рябь по контуру — то, чем
/// натёкшая жидкость отличается от нарисованной циркулем.
const POOL_WAVES: [(f32, f32); 3] = [(5.0, 0.028), (9.0, 0.018), (14.0, 0.010)];
/// Сколько ручейков уходит из лужи. Круглое пятно на карте — это что угодно;
/// пятно с потёком — только кровь.
const POOL_RIVULETS: (usize, usize) = (2, 4);
/// Докуда добегает ручеёк, в долях полуячейки. Задан **концом**, а не длиной:
/// длиной он тонул в самой луже (доли достают до 0,5–0,8), и наружу торчал
/// огрызок в пару пикселей.
const POOL_RIVULET_REACH: (f32, f32) = (0.58, 0.78);
/// Радиус ручейка у истока; к устью он сходит на нет.
const POOL_RIVULET_R: (f32, f32) = (0.050, 0.090);
/// Увод ручейка вбок на середине, в долях полуячейки: прямая струйка выглядит
/// начерченной по линейке.
const POOL_RIVULET_SWAY: f32 = 0.09;

// --- Брызги ---
/// Сколько капель в веере. Много и мелких: десяток крупных читается не
/// брызгами, а горстью зёрен.
const SPATTER_DROPS: (usize, usize) = (34, 56);
/// Сколько капель отброшено назад, против веера.
const SPATTER_BACK: (usize, usize) = (3, 7);
/// Крупные капли у самой лужи — те, что стекли, а не долетели.
const SPATTER_SATELLITES: (usize, usize) = (2, 5);
/// Ближе этого капли не ложатся: там лужа, и они в ней тонут.
const SPATTER_NEAR: f32 = 0.22;
/// Дальше этого капля вышла бы за ячейку.
const SPATTER_FAR: f32 = 0.84;
/// Полураствор веера у тела и на излёте, радиан.
const SPATTER_FAN: (f32, f32) = (0.35, 0.95);
/// Радиус капли у тела и на излёте: дальние мельче.
const SPATTER_NEAR_R: (f32, f32) = (0.016, 0.042);
const SPATTER_FAR_R: (f32, f32) = (0.007, 0.020);
/// Хвост капли, **в радиусах самой капли** и тем длиннее, чем дальше она
/// улетела: упавшая под углом капля — не круг, а запятая, и её узкий конец
/// смотрит по полёту. В долях ячейки хвост задавать нельзя — пробовали, и
/// мелкая дальняя капля выходила иглой в десяток своих поперечников.
const SPATTER_TAIL: (f32, f32) = (0.0, 1.7);
/// Во сколько раз конец хвоста тоньше головы капли.
const SPATTER_TAIL_TAPER: f32 = 0.45;
/// Доля жирных капель и во сколько раз они крупнее: ровный по калибру веер
/// выглядит напылением через трафарет, а не кровью.
const SPATTER_FAT_SHARE: f32 = 0.14;
const SPATTER_FAT: f32 = 2.1;
/// Радиус капли-спутника у лужи.
const SPATTER_SATELLITE_R: (f32, f32) = (0.028, 0.060);

/// Разыгрыватель формы варианта: тот же splitmix64, что раздаёт позы трупам,
/// только сеется номером варианта, а не сущностью.
struct Seq(u64);

impl Seq {
    fn new(seed: u64) -> Self {
        Self(splitmix64(seed ^ 0x_b100_d5ee_d000_0001))
    }

    /// Следующее число в 0…1 (единицы не достигает).
    fn unit(&mut self) -> f32 {
        self.0 = splitmix64(self.0);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, (from, to): (f32, f32)) -> f32 {
        from + (to - from) * self.unit()
    }

    /// Целое из отрезка, оба конца включительно.
    fn count(&mut self, (from, to): (usize, usize)) -> usize {
        from + (self.unit() * (to - from + 1) as f32) as usize
    }

    fn angle(&mut self) -> f32 {
        self.unit() * TAU
    }

    /// Симметричный разброс −1…1.
    fn spread(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

fn dir(angle: f32) -> Vec2 {
    Vec2::from_angle(angle)
}

/// Мягкий минимум: там, где два контура сходятся ближе `k`, между ними
/// вырастает перешеек вместо угла. `k = 0` — обычный минимум, то есть
/// объединение без слипания (брызги).
fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = ((k - (a - b).abs()) / k).clamp(0.0, 1.0);
    a.min(b) - h * h * k * 0.25
}

/// Пятно крови: капсулы в координатах ячейки (−1…1), мягкость их слипания и
/// гармоники кромки.
pub struct Stain {
    blobs: Vec<Capsule>,
    weld: f32,
    waves: [(f32, f32, f32); POOL_WAVES.len()],
}

impl Stain {
    /// Лужа номер `variant`: слипшиеся доли и ручеёк-другой из них.
    pub fn pool(variant: usize) -> Self {
        let mut seq = Seq::new(variant as u64);
        let core = seq.range(POOL_CORE);
        let stretch = seq.range(POOL_STRETCH);
        let tilt = Vec2::from_angle(seq.angle());
        // направление под номером доли, уже вытянутое эллипсом и повёрнутое
        let spread =
            |angle: f32| tilt.rotate(Vec2::new(angle.cos() * stretch, angle.sin() / stretch));
        let mut blobs = vec![Capsule::dot(Vec2::ZERO, core)];

        let lobes = seq.count(POOL_LOBES);
        let step = TAU / lobes as f32;
        for lobe in 0..lobes {
            // доли расставлены по обводу с джиттером, а не свалены случайно:
            // случайные центры сбиваются в комок, и лужа выходит круглой
            let angle = step * (lobe as f32 + 0.5 * seq.spread());
            let centre = spread(angle) * seq.range(POOL_LOBE_DIST);
            blobs.push(Capsule::dot(centre, seq.range(POOL_LOBE_RADIUS)));
        }

        for _ in 0..seq.count(POOL_RIVULETS) {
            let angle = seq.angle();
            let radius = seq.range(POOL_RIVULET_R);
            let source = dir(angle) * core * 0.8;
            let mouth = dir(angle) * seq.range(POOL_RIVULET_REACH);
            let knee =
                source.lerp(mouth, 0.55) + dir(angle).perp() * POOL_RIVULET_SWAY * seq.spread();
            blobs.push(Capsule::new(source, knee, radius, radius * 0.75));
            blobs.push(Capsule::new(knee, mouth, radius * 0.75, radius * 0.4));
        }

        let waves = std::array::from_fn(|wave| {
            let (order, amplitude) = POOL_WAVES[wave];
            (order, amplitude, seq.angle())
        });
        Self {
            blobs,
            weld: POOL_WELD,
            waves,
        }
    }

    /// Брызги номер `variant`: веер капель, редкий отброс назад и несколько
    /// крупных спутников у самой лужи.
    pub fn spatter(variant: usize) -> Self {
        let mut seq = Seq::new(0x_5eed_0000 + variant as u64);
        let axis = seq.angle();
        let mut blobs = Vec::new();

        let drops = seq.count(SPATTER_DROPS);
        for _ in 0..drops {
            // разлёт смещён к телу: у ног густо, на излёте редкие одиночки
            let flight = seq.unit().powf(1.6);
            let fan = SPATTER_FAN.0 + (SPATTER_FAN.1 - SPATTER_FAN.0) * flight;
            let angle = axis + fan * seq.spread();
            let reach = SPATTER_NEAR + (SPATTER_FAR - SPATTER_NEAR) * flight;
            let head = dir(angle) * reach;
            let mut radius = lerp_range(SPATTER_NEAR_R, SPATTER_FAR_R, flight, &mut seq);
            if seq.unit() < SPATTER_FAT_SHARE {
                radius *= SPATTER_FAT;
            }
            let tail = radius * seq.range(SPATTER_TAIL) * flight;
            blobs.push(Capsule::new(
                head,
                head + dir(angle) * tail,
                radius,
                radius * SPATTER_TAIL_TAPER,
            ));
        }

        for _ in 0..seq.count(SPATTER_BACK) {
            let angle = axis + std::f32::consts::PI + SPATTER_FAN.0 * seq.spread();
            let reach = seq.range((SPATTER_NEAR, SPATTER_NEAR + 0.30));
            blobs.push(Capsule::dot(dir(angle) * reach, seq.range(SPATTER_FAR_R)));
        }

        for _ in 0..seq.count(SPATTER_SATELLITES) {
            let angle = seq.angle();
            let reach = seq.range((SPATTER_NEAR, SPATTER_NEAR + 0.22));
            blobs.push(Capsule::dot(
                dir(angle) * reach,
                seq.range(SPATTER_SATELLITE_R),
            ));
        }

        Self {
            blobs,
            // капли не слипаются: две рядом — две капли, а не гантель
            weld: 0.0,
            waves: [(0.0, 0.0, 0.0); POOL_WAVES.len()],
        }
    }

    /// Насколько глубоко точка `p` ячейки (−1…1) сидит внутри пятна;
    /// отрицательное — снаружи.
    fn inside(&self, p: Vec2) -> f32 {
        let mut distance = f32::MAX;
        for blob in &self.blobs {
            let (centre, bound) = blob.bound();
            if p.distance_squared(centre) > (bound + self.weld + CULL_SLACK).powi(2) {
                continue;
            }
            distance = smin(distance, blob.distance(p), self.weld);
        }
        let theta = p.y.atan2(p.x);
        let ripple: f32 = self
            .waves
            .iter()
            .map(|(order, amplitude, phase)| amplitude * (order * theta + phase).sin())
            .sum();
        -distance + ripple
    }

    /// Тексель пятна в точке `p` ячейки (−1…1): яркость (множитель к цвету
    /// спрайта) и альфа. `edge` — полуширина сглаживания в единицах ячейки.
    pub fn texel(&self, p: Vec2, edge: f32) -> (f32, f32) {
        let inside = self.inside(p);
        let coverage = smoothstep(-edge, edge, inside);
        let depth = (inside / DEPTH).clamp(0.0, 1.0).sqrt();
        (
            1.0 - (1.0 - CORE_SHADE) * depth,
            coverage * (FILM + (1.0 - FILM) * depth),
        )
    }
}

/// Радиус капли на доле полёта `flight`: отрезок разброса сам едет от
/// «у тела» к «на излёте», и уже внутри него бросается кость.
fn lerp_range(near: (f32, f32), far: (f32, f32), flight: f32, seq: &mut Seq) -> f32 {
    seq.range((
        near.0 + (far.0 - near.0) * flight,
        near.1 + (far.1 - near.1) * flight,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const EDGE: f32 = 2.0 / 128.0;

    /// Все пятна атласа подряд.
    fn every_stain() -> impl Iterator<Item = (String, Stain)> {
        (0..POOLS)
            .map(|variant| (format!("pool #{variant}"), Stain::pool(variant)))
            .chain(
                (0..SPATTERS)
                    .map(|variant| (format!("spatter #{variant}"), Stain::spatter(variant))),
            )
    }

    /// Доля ячейки, закрытая пятном плотнее половины альфы.
    fn coverage(stain: &Stain) -> f32 {
        let n = 96;
        let mut covered = 0;
        for y in 0..n {
            for x in 0..n {
                let p = (Vec2::new(x as f32, y as f32) + 0.5) / n as f32 * 2.0 - 1.0;
                if stain.texel(p, EDGE).1 > 0.5 {
                    covered += 1;
                }
            }
        }
        covered as f32 / (n * n) as f32
    }

    /// Кровь не имеет права вылезти за свою ячейку: соседняя в атласе — чужой
    /// глиф, и на глубоких мипах он уже в двух текселях. Считается по
    /// габаритам капсул, а не по выборке текселей: выборка мимо одинокой
    /// капли проходит молча.
    #[test]
    fn every_stain_stays_inside_its_cell() {
        for (name, stain) in every_stain() {
            let waves: f32 = stain.waves.iter().map(|(_, amplitude, _)| amplitude).sum();
            for blob in &stain.blobs {
                // дальняя точка капсулы — на одном из двух её концов;
                // габаритный круг ([`Capsule::bound`]) для этого слишком груб,
                // он завышает косо стоящий ручеёк на пол-его длины.
                // `smin` выносит контур наружу ещё на k/4, гармоники — на свою
                // сумму
                let reach = (blob.a.length() + blob.ra).max(blob.b.length() + blob.rb)
                    + stain.weld / 4.0
                    + waves;
                assert!(reach < 0.93, "{name} reaches {reach} of its half-cell");
            }
        }
    }

    /// Лужа — сплошное пятно с густой серединой и тонкой кромкой.
    #[test]
    fn a_pool_is_thick_in_the_middle_and_a_film_at_its_rim() {
        for variant in 0..POOLS {
            let pool = Stain::pool(variant);
            let (core_shade, core_alpha) = pool.texel(Vec2::ZERO, EDGE);
            assert_eq!(core_alpha, 1.0, "pool #{variant} has a hole in the middle");
            assert!(
                (core_shade - CORE_SHADE).abs() < 1e-5,
                "pool #{variant} is not darkest at its core: {core_shade}"
            );

            // точка ровно на кромке: альфа заметно меньше единицы, а яркость
            // выше, чем в середине, — плёнка светлее густого
            let rim = (0..950)
                .map(|step| Vec2::X * step as f32 / 1000.0)
                .find(|p| (0.05..0.95).contains(&pool.texel(*p, EDGE).1))
                .expect("у лужи есть кромка");
            let (rim_shade, _) = pool.texel(rim, EDGE);
            assert!(
                rim_shade > core_shade,
                "pool #{variant} rim {rim_shade} is not lighter than core {core_shade}"
            );

            let share = coverage(&pool);
            assert!(
                (0.10..0.50).contains(&share),
                "pool #{variant} covers {share} of its cell"
            );
        }
    }

    /// Брызги — именно брызги: капель много, они врозь, а середина ячейки
    /// пуста (там стоит лужа своим спрайтом).
    #[test]
    fn a_spatter_is_a_scatter_of_drops_around_an_empty_middle() {
        for variant in 0..SPATTERS {
            let spatter = Stain::spatter(variant);
            assert_eq!(
                spatter.texel(Vec2::ZERO, EDGE).1,
                0.0,
                "spatter #{variant} fills the middle, where the pool goes"
            );
            assert!(
                spatter.blobs.len() >= SPATTER_DROPS.0,
                "spatter #{variant} has too few drops"
            );
            let share = coverage(&spatter);
            assert!(
                (0.002..0.12).contains(&share),
                "spatter #{variant} covers {share} of its cell"
            );
        }
    }

    /// Варианты обязаны быть разными: тел на карте тысячи, и повтор формы
    /// виден раньше, чем что-либо ещё. Сравниваются сами капсулы, а не площадь
    /// пятна: площадь — одно число, и у двух непохожих луж оно совпало
    /// (0,2169 у #0 и #6). Похожи ли они **на глаз** — вопрос к витрине
    /// (`examples/demos/blood_gallery`), а здесь держится то, что проверяемо:
    /// жребий до формы доходит и одинаковых наборов не выдаёт.
    #[test]
    fn no_two_stains_are_rolled_alike() {
        let shape = |stain: &Stain| -> Vec<u32> {
            stain
                .blobs
                .iter()
                .flat_map(|blob| {
                    [blob.a.x, blob.a.y, blob.b.x, blob.b.y, blob.ra, blob.rb].map(f32::to_bits)
                })
                .collect()
        };
        let shapes: Vec<(String, Vec<u32>)> = every_stain()
            .map(|(name, stain)| (name, shape(&stain)))
            .collect();
        for (index, (left, left_shape)) in shapes.iter().enumerate() {
            for (right, right_shape) in &shapes[index + 1..] {
                assert_ne!(left_shape, right_shape, "{left} is a copy of {right}");
            }
        }
    }

    /// Мягкий минимум сливает близкие контуры и не трогает далёкие.
    #[test]
    fn smin_welds_what_is_close_and_leaves_the_rest() {
        assert_eq!(smin(0.3, 0.5, 0.0), 0.3);
        assert_eq!(smin(0.3, 0.9, 0.1), 0.3);
        assert!(smin(0.3, 0.3, 0.2) < 0.3);
    }
}
