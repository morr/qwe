# arch/kerb-views — задача 03: типизированные выходы corners

Пилот: выход `KerbReturns` (`src/map/roads/corners.rs`) — мешок из 11 pub Vec —
заменён типизированной выходной поверхностью; порядок укладки слоёв — теперь
инвариант модуля, а не вызывающего. Поведение заморожено: та же геометрия, те же
слои, тот же порядок, тот же z-стек. NodePaint — волна 2, не тронут.

## Что сделано

- `KerbReturns::lay(layers: KerbLayers, meadows, scraps)` — единственный метод
  укладки, внутри corners.rs, по образцу рук build_roads дословно: углы газонов →
  лоскуты `Scraps`, затем веером streets/alleys → unpaved → sidewalks (+overlaps) →
  verges → verge lawns (meadows решает lawn/yard), последними noses+bends
  (`push_polygon`). `KerbLayers` — именованная связка семи `&mut MeshBuilder`.
- Представления-читатели (шаблон — `AsphaltBreaks`/`PaintBreaks` в node_paint.rs):
  - `KerbPaving` (`KerbReturns::paving()`) — чем режутся лоскуты газона:
    `flat()` (roads-Street + sidewalks + verges) и `curved()` (noses/bends с
    Fill Street|Sidewalk) раздельно, чтобы вызывающий вставлял gore_outlines
    между ними — `Scraps` чувствителен к порядку вставки (Grid: insertion order
    inside a cell). Наезды плитки (`overlaps`) в мощение не входят.
  - `KerbStreets` (`KerbReturns::streets()`) — асфальт улиц для горл штриховки
    (`gores::mouths`): roads-Street + noses/bends-Road(Street).
- Поля `KerbReturns`: `pub` → `pub(super)` — читаются внутри `map::roads`
  (отчёт RoadReport и старые тесты). Волна 2 переведёт тесты на представления и
  закроет поля совсем.
- `build_roads`: семь рук укладки → один вызов `lay`; читатели мощения и горл —
  на представления; замыкание `paved_fill` ушло в `KerbPaving::curved`.

## Коммиты

1. `lay kerb returns into layers from corners, close the fields to roads` —
   поверхность: lay + KerbLayers + представления + pub(super).
2. `read kerb paving and street asphalt through corners views` — читатели.
3. `test the kerb returns views and laying` — 4 новых теста.
4. `fmt the kerb views tests` — прогон rustfmt по тестам.
5. `document the kerb returns laying order` — CONTEXT.md, roads.md, NOTES.md.

## Червячки инструментария (найдены по ходу)

- `tools/gallery_batch` ломается на rev со слэшем (`arch/kerb-views`):
  имя лога `run_N_<rev>.log` превращается в путь с несуществующим каталогом —
  строка молча пропускается (CHECKOUT FAILED). Обход: хеш коммита в tasks.
- Витрина (`cargo run --example roads`) не создаёт каталоги кадров — путь в
  манифесте обязан существовать (IO error в логе, кадр потерян).
- Сырые PNG витрины нельзя сравнивать байтами: в кадр вшита панель статистики
  с таймингами прогона и `png:tIME`. Сравнивать кропы (+866+36) по AE.

## Верификация

- Новые тесты (через представления, не поля): `lay_routes_each_kerb_group_to_
  its_layer`, `a_meadow_lawn_takes_the_lawn_layer_over_the_yard`,
  `paving_cuts_lawns_with_street_asphalt_but_not_tile_overlaps`,
  `streets_view_yields_only_street_class_asphalt` — зелёные (42 corners-теста
  всего).
- `tools/check.sh src/map/roads.rs src/map/roads/corners.rs` — check: OK
  (1451 тест, clippy -D warnings, fmt).
- gallery_batch, Тула, 10 кербовых кадров витрины (01 крестовина, 05 звезда,
  06 Y-остров, 09 изгиб/R26, 12 улица в половину, 15 косой, 19 смещённые
  примыкания, 20 кольцо из дуг/R26, 23 S-кривая (Путейская, N4), 28 сужение
  (R37-04)), scale 2, сравнение кропов (+866+36, 1820x1820):
  - **8 из 10 кадров: AE=0, пиксели идентичны** и против master (80ae5823), и
    против точки ветвления 9f289f8c. Байтовые различия сырых PNG — только
    `png:tIME` (время снимка) и текстовая панель статистики витрины слева
    (тайминги прогона), кропом срезается.
  - **2 кадра (23_s_curve, 28_narrowing_t): внутри шума повтора.** Витрина
    недетерминирована от запуска к запуску на этих кадрах: повтор ТОГО ЖЕ
    коммита master даёт AE=3.46M (23) и AE=11.0M (28) — дрожь успокоения
    камеры (±1 уровень на весь кадр, MAE 1.05), тогда как master против ветки
    даёт AE=1.24M/3.71M (MAE 0.38/1.12) — меньше, чем повтор против себя.
    Вклад ветки не отличим от нуля.
  - Отдельно: master на 1 коммит (перенос smoothstep) впереди точки
    ветвления — на всех 8 точных кадрах пиксельно нейтрален.
- Перф: перестановка кода без новой работы; представления — тонкие обёртки
  над срезами, итераторы те же. Замер не нужен.

## Осознанные решения

- `fork_gores` кладётся на прежнем месте (`gores.add_forks` в build_roads):
  `Gores` строится из нарисованного порядка ПОСЛЕ блока укладки кербов —
  втянуть fork gores в `lay` значило бы двигать ~40 строк build_roads и менять
  порядок push в билдерах. Задокументировано в lay/roads.md.
- Счётчики RoadReport (`kerb_returns`, `sidewalk_returns`, `outer_corners`, …)
  читают поля напрямую: это диагностика, не геометрия; тоже волна 2.
- «Контекст» и «укладка» из плана серии слились в один коммит 1 — резать
  corners.rs на два никогда не компилировавшихся промежуточных состояния дороже,
  чем worth it.

## Волна 2 (следующему исполнителю)

- **NodePaint** (`src/map/roads/node_paint.rs`): junctions/pockets/zebras/
  stop_lines по образцу AsphaltBreaks/PaintBreaks/LineBreaks (укладка/чтение
  через представления, поля закрыть).
- Конвертация старых тестов на представления: `src/map/roads/tests.rs`
  (строки ~3157, 4429, 4538, 4600), `src/map/roads/merges/tests.rs` (~128),
  внутренние тесты corners.rs (~2197+). После этого поля `KerbReturns` —
  private, а не pub(super).
- Рассмотреть: втягивание `add_forks` в `lay`, если `Gores` удастся строить до
  укладки кербов без переноса push-порядка.
