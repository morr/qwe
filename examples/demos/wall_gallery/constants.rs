//! Константы стены — как они записаны в шейдере, а не их копия.
//!
//! Числа, которыми задан рисунок стены, живут в двух местах. **Проёмы и швы**
//! — в `roof.wgsl`, в разделе после заголовка [`WALL_SECTION`]: доли ячейки
//! под окно, балкон, дверь, толщина рамы и импоста, шаг ряда кладки и ребра
//! профлиста. **Кто чем облицован и кому положены балконы** — в `layers.rs` и
//! `material.rs`: пороги этажности и ширины стены.
//!
//! Витрина печатает их рядом с картинкой — иначе «тут окно шире» и «балкон
//! сюда не встал» не с чем сопоставить, и подобрать число по виду нельзя, не
//! переключаясь на редактор.
//!
//! Берутся они **из самих файлов** (`include_str!`), а не повторяются числом:
//! копия разошлась бы с оригиналом в первую же правку, и витрина начала бы
//! врать ровно про то, ради чего её открыли.
//!
//! Разбор нарочно наивный, строка вида `const ИМЯ: f32 = ЧИСЛО;`. Коды
//! материалов отсеиваются по типу — они `u32`, зеркало `WallKind::code`, а не
//! настройка.

/// Текст шейдера, вшитый в витрину при сборке. Кровля и стена живут в одном
/// файле — делит их заголовок раздела.
const SHADER_SOURCE: &str = include_str!("../../../assets/shaders/roof.wgsl");

/// Исходник правил облицовки — оттуда же и тем же способом.
const LAYERS_SOURCE: &str = include_str!("../../../src/map/buildings/layers.rs");

/// Высота этажа лежит не в правилах облицовки, а среди общих единиц: по ней
/// парсер переводит `building:levels` в метры, а стена переводит обратно, и
/// объявлена она поэтому одна на всех (`settings.rs`). Читается так же.
const SETTINGS_SOURCE: &str = include_str!("../../../src/settings.rs");

/// Заголовок стенной половины шейдера. Зеркало того же разреза в
/// `roof_gallery/constants.rs`: та витрина берёт всё **до** него, эта — всё
/// **после**, и вместе они покрывают файл ровно один раз.
const WALL_SECTION: &str = "─── стена";

/// Константы стены из шейдера, в порядке файла. Значение — тем же текстом, что
/// и в коде: `0.46`, а не `0.460000`.
pub(crate) fn shader_constants() -> Vec<(&'static str, &'static str)> {
    SHADER_SOURCE
        .lines()
        .skip_while(|line| !line.contains(WALL_SECTION))
        .filter_map(|line| parse_const(line, ": f32 = "))
        .collect()
}

/// Пороги, по которым стене достаются балконы, и метрика ячейки — из
/// `layers.rs` и `settings.rs`. Без них картинка не отвечает на «почему на этом
/// доме их нет».
pub(crate) fn rule_constants() -> Vec<(&'static str, &'static str)> {
    const SHOWN: [&str; 4] = [
        "PANEL_WIDTH",
        "STOREY_HEIGHT",
        "BALCONY_STOREYS_MIN",
        "BALCONY_COLUMNS_MIN",
    ];
    LAYERS_SOURCE
        .lines()
        .chain(SETTINGS_SOURCE.lines())
        .filter_map(|line| parse_const(line, ": f32 = "))
        .filter(|(name, _)| SHOWN.contains(name))
        .collect()
}

fn parse_const<'a>(line: &'a str, kind: &str) -> Option<(&'a str, &'a str)> {
    // видимость перед `const` бывает любая (`pub`, `pub(super)`) и к значению
    // отношения не имеет
    let declaration = line
        .trim()
        .split_once("const ")
        .filter(|(before, _)| before.is_empty() || before.starts_with("pub"))
        .map(|(_, declaration)| declaration)?;
    let (name, value) = declaration.split_once(kind)?;
    Some((name, value.strip_suffix(';')?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Разбор держится на форме строки в чужом файле, поэтому пустой список —
    /// не «констант нет», а «формат разъехался»: без этой проверки витрина
    /// молча покажет пустую группу.
    ///
    /// Тесты примеров `cargo test` не запускает (он их только собирает), так
    /// что этот идёт под `cargo test --examples`.
    #[test]
    fn the_wall_constants_are_read_from_the_shader() {
        let constants = shader_constants();
        for expected in ["WINDOW_WIDE", "BALCONY_FILLED", "BRICK_COURSE"] {
            assert!(
                constants.iter().any(|(name, _)| *name == expected),
                "в стенной половине `roof.wgsl` не нашлось {expected}: {constants:?}"
            );
        }
        // кровельная половина того же файла сюда попадать не должна
        assert!(
            !constants.iter().any(|(name, _)| *name == "PATCH_CELL"),
            "константы кровли — не стенные: {constants:?}"
        );
    }

    /// То же и по той же причине для порогов балконов. `STOREY_HEIGHT` лежит в
    /// другом файле, чем остальные три, — без него из группы молча пропала бы
    /// строка, а не появилась бы ошибка.
    #[test]
    fn the_balcony_rules_are_read_from_the_source() {
        let constants = rule_constants();
        for expected in ["PANEL_WIDTH", "BALCONY_STOREYS_MIN", "STOREY_HEIGHT"] {
            assert!(
                constants.iter().any(|(name, _)| *name == expected),
                "в исходниках правил не нашлось {expected}: {constants:?}"
            );
        }
    }
}
