# SPDX-FileCopyrightText: 2026 Yangtse Su <yangtsesu@gmail.com>
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The zh-CN catalog (Simplified Chinese). It carries exactly the keys of `locales/en-US/main.ftl`;
# `tests/i18n.rs` compares the two key sets, so a key added on one side without the other is a
# failing test rather than an English string in Chinese output.
#
# Wording follows mainland-Chinese weather-forecast usage ("晴", "多云", "东南风"), and the line is
# kept short on purpose: the art table gives a label ten display columns, so a long phrase would be
# clipped where a short one reads whole.

# --- Conditions: WMO 4677 ---------------------------------------------------------------

cond-0 = 晴
cond-1 = 晴间多云
cond-2 = 多云
cond-3 = 阴
cond-4 = 烟
cond-5 = 霾
cond-6 = 浮尘
cond-7 = 扬沙
cond-8 = 未知
cond-9 = 未知
cond-10 = 轻雾
cond-11 = 未知
cond-12 = 未知
cond-13 = 未知
cond-14 = 未知
cond-15 = 未知
cond-16 = 未知
cond-17 = 未知
cond-18 = 未知
cond-19 = 未知
cond-20 = 未知
cond-21 = 未知
cond-22 = 未知
cond-23 = 未知
cond-24 = 未知
cond-25 = 未知
cond-26 = 未知
cond-27 = 未知
cond-28 = 未知
cond-29 = 未知
cond-30 = 未知
cond-31 = 未知
cond-32 = 未知
cond-33 = 未知
cond-34 = 未知
cond-35 = 未知
cond-36 = 未知
cond-37 = 未知
cond-38 = 未知
cond-39 = 未知
cond-40 = 未知
cond-41 = 未知
cond-42 = 未知
cond-43 = 未知
cond-44 = 未知
cond-45 = 雾
cond-46 = 未知
cond-47 = 未知
cond-48 = 雾凇
cond-49 = 未知
cond-50 = 未知
cond-51 = 毛毛雨
cond-52 = 未知
cond-53 = 小雨
cond-54 = 未知
cond-55 = 大雨
cond-56 = 冻毛毛雨
cond-57 = 强冻毛毛雨
cond-58 = 未知
cond-59 = 未知
cond-60 = 未知
cond-61 = 小雨
cond-62 = 未知
cond-63 = 中雨
cond-64 = 未知
cond-65 = 大雨
cond-66 = 冻雨
cond-67 = 强冻雨
cond-68 = 未知
cond-69 = 未知
cond-70 = 未知
cond-71 = 小雪
cond-72 = 未知
cond-73 = 中雪
cond-74 = 未知
cond-75 = 大雪
cond-76 = 未知
cond-77 = 米雪
cond-78 = 未知
cond-79 = 冰粒
cond-80 = 小阵雨
cond-81 = 阵雨
cond-82 = 暴雨
cond-83 = 未知
cond-84 = 未知
cond-85 = 小阵雪
cond-86 = 大阵雪
cond-87 = 未知
cond-88 = 未知
cond-89 = 未知
cond-90 = 未知
cond-91 = 未知
cond-92 = 未知
cond-93 = 未知
cond-94 = 未知
cond-95 = 雷暴
cond-96 = 雷暴伴小冰雹
cond-97 = 强雷暴
cond-98 = 未知
cond-99 = 雷暴伴大冰雹
cond-unknown = 未知

# --- Day parts --------------------------------------------------------------------------
# Short names: they sit in a ten-column label, and the layout degrades gracefully when the
# terminal is narrow, so two glyphs (four columns) fit where the full 早晨/中午/傍晚/夜间 would not.

part-morning = 早上
part-noon = 中午
part-evening = 傍晚
part-night = 夜间

# --- Calendar names ---------------------------------------------------------------------

weekday-mon = 周一
weekday-tue = 周二
weekday-wed = 周三
weekday-thu = 周四
weekday-fri = 周五
weekday-sat = 周六
weekday-sun = 周日

month-1 = 1月
month-2 = 2月
month-3 = 3月
month-4 = 4月
month-5 = 5月
month-6 = 6月
month-7 = 7月
month-8 = 8月
month-9 = 9月
month-10 = 10月
month-11 = 11月
month-12 = 12月

# The ISO form stays ISO in every catalog: it is what `%d` promises.
date-iso = { $year }-{ $month-number }-{ $day }
# Chinese writes the parts largest to smallest, without separators, and never zero pads a day.
date-short = { $month }{ $day-plain }日 { $weekday }
date-today = 今天 { $month }{ $day-plain }日

# --- Report and record labels -----------------------------------------------------------

label-report = 天气报告：
label-data = 数据：

# 观测类报告在当前实况下方打印的一行，以及说明为何没有逐日表格的页脚。
label-observed = 观测
format-age-minutes = { $minutes } 分钟前
format-age-hours = { $hours } 小时前
note-no-forecast = 无预报：{ $provider } 为实时观测数据

# The `plain` record keys. They keep their meaning, and the line keeps its shape.
label-location = 地点
label-updated = 更新
label-current = 当前
label-day = 逐日
label-attribution = 来源

# --- Measurements -----------------------------------------------------------------------

label-feels = 体感
label-wind = 风
label-humidity = 湿度
label-precip = 降水
label-pressure = 气压
label-visibility = 能见度
label-uv = 紫外线
label-sunrise = 日出
label-sunset = 日落

# --- Values the renderers name ------------------------------------------------------------

uv-band-low = 弱
uv-band-moderate = 中等
uv-band-high = 强
uv-band-very-high = 很强
uv-band-extreme = 极强

na = 无数据
moon-na = 无数据

# The compass rose: the sixteen points, each spelled with the direction the wind blows from.
# The arrow is the renderer's, so it is not repeated here.
dir-n = 北风
dir-nne = 北东北风
dir-ne = 东北风
dir-ene = 东东北风
dir-e = 东风
dir-ese = 东东南风
dir-se = 东南风
dir-sse = 南东南风
dir-s = 南风
dir-ssw = 南西南风
dir-sw = 西南风
dir-wsw = 西西南风
dir-w = 西风
dir-wnw = 西西北风
dir-nw = 西北风
dir-nnw = 北西北风

# --- Formatting: one converted value into its display string ------------------------------
# Chinese writes the unit right after the number, like the English compact style.

format-temp-c = { $value }°C
format-temp-f = { $value }°F
format-wind-kmh = { $value }km/h
format-wind-mph = { $value }mph
format-wind-knots = { $value }kn
format-wind-mps = { $value }m/s
format-pressure-hpa = { $value }hPa
format-pressure-inhg = { $value }inHg
format-pressure-mmhg = { $value }mmHg
format-distance-km = { $value }km
format-distance-mi = { $value }mi
format-humidity = { $value }%
format-uv = { $value }（{ $band }）
format-precip-mm = { $value }mm
format-precip-in = { $value }in

# --- 警报（步骤 15） -------------------------------------------------------------------------
# CAP 的严重度三元组、来源名称与横幅片段。alert-banner-line 是所有格式中每条警报的首行；
# 图标与颜色由渲染器添加，因此翻译只决定文字和标点。WMO SWIC 与 FPAS 的署名行是它们的
# 使用条款要求在警报旁显示的内容。

label-alert = 警报

alert-none = 无生效中的天气警报
alert-banner-line = { $event } — { $severity }
alert-until = 持续至 { $time }
alert-since = 自 { $time } 起
alert-more-count = …另有 { $count } 条

alert-severity-unknown = 未知
alert-severity-minor = 轻度
alert-severity-moderate = 中度
alert-severity-severe = 严重
alert-severity-extreme = 极端

alert-urgency-unknown = 未知
alert-urgency-past = 已过去
alert-urgency-future = 未来
alert-urgency-expected = 预计
alert-urgency-immediate = 立即

alert-certainty-unknown = 未知
alert-certainty-unobserved = 未观测
alert-certainty-possible = 可能
alert-certainty-unlikely = 不太可能
alert-certainty-likely = 很可能
alert-certainty-observed = 已观测

alert-source-nws = 美国国家气象局
alert-source-meteoalarm = MeteoAlarm
alert-source-qweather = 和风天气
alert-source-hko = 香港天文台
alert-source-wmoswic = WMO 严重天气信息中心
alert-source-fpas = FOSS 公共警报服务器
alert-source-visualcrossing = Visual Crossing

alert-credit-wmoswic = 警报由 WMO 严重天气信息中心（severeweather.wmo.int）提供，© 各发布机构
alert-credit-fpas = 警报经由 FOSS 公共警报服务器（{ $host }）提供

# --- 空气质量（步骤 16） -------------------------------------------------------------------------
# 面板标签、两套 AQI 分级名称以及污染物/花粉名称。分级名称按 `AqiCategory::index` 索引；
# `aqi-no-coverage` 表示该地点不在花粉预报范围内；`aqi-uv-source` 标明面板中的紫外线数值
# 来自天气数据而非空气质量测量。

aqi-panel-title = 空气质量
aqi-us-label = 美国 AQI
aqi-european-label = 欧洲 AQI
aqi-pollen-title = 花粉
aqi-no-coverage = 该地点未覆盖
aqi-unavailable = 空气质量数据不可用
aqi-uv-label = 紫外线
aqi-uv-source = 天气数据

aqi-category-good = 良好
aqi-category-moderate = 中度
aqi-category-unhealthy-sensitive = 敏感人群不健康
aqi-category-unhealthy = 不健康
aqi-category-very-unhealthy = 非常不健康
aqi-category-hazardous = 危险
aqi-category-fair = 尚可
aqi-category-poor = 较差
aqi-category-very-poor = 很差
aqi-category-extremely-poor = 极差

pm2-5 = PM2.5
pm10 = PM10
o3 = O3
no2 = NO2
so2 = SO2
co = CO

pollen-alder = 桤木
pollen-birch = 桦树
pollen-grass = 禾本科
pollen-mugwort = 蒿草
pollen-olive = 橄榄
pollen-ragweed = 豚草

unit-ug-m3 = 微克/立方米
unit-grains-m3 = 粒/立方米
