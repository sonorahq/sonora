# common
common-left = 左
common-right = 右
common-search = 搜索
common-unknown = 未知
common-not-provided = 未提供
common-not-available = 不可用
common-unavailable = { $name }（不可用）
common-cancel = 取消
common-save = 保存
common-delete = 删除
common-play = 播放
common-more = 更多
common-previous = 上一个
common-next = 下一个
common-dismiss = 忽略
common-clear = 清空
common-undo = 撤销
common-redo = 重做
common-cut = 剪切
common-copy = 复制
common-paste = 粘贴
common-show-all = 显示全部
common-show-less = 收起
common-local-files = 本地文件
common-server-address = 服务器地址
common-api-key = API 密钥
common-not-now = 以后再说
common-not-connected = 未连接
common-sonora = Sonora
common-more-count = +{ $count }
number-group = { "," }

# units
unit-percent = { $value }%
unit-db = { $value } dB
unit-hz = { $value } Hz
unit-khz = { $value } kHz
unit-kbps = { $value } kbps
unit-gb = { $value } GB
unit-mb = { $value } MB
unit-fps = { $value } FPS
unit-px = { $value } px
runtime-seconds = { $seconds } 秒
runtime-minutes = { $minutes } 分 { $seconds } 秒
runtime-hours = { $hours } 小时 { $minutes } 分

# navigation
nav-history = 历史记录
nav-home = 首页
nav-search = 搜索
nav-library = 你的音乐库
nav-settings = 设置
nav-songs = 歌曲
nav-albums = 专辑
nav-playlists = 播放列表
nav-artists = 歌手
nav-local = 本地音乐
nav-back = 返回
nav-forward = 前进
nav-sidebar = 切换侧边栏
nav-sidebar-right = 显示或隐藏歌词和播放队列
nav-pinned = 已固定
nav-unpin = 取消固定
nav-pin-hint = 拖放到此处以固定
library-liked-songs = 收藏
library-no-songs = 还没有收藏的歌曲
library-no-albums = 还没有保存的专辑
library-no-playlists = 还没有播放列表
library-no-artists = 还没有收藏的歌手
library-no-local-songs = 未找到导入的歌曲
library-no-local-albums = 未找到导入的专辑
library-no-local-artists = 未找到导入的歌手
library-no-local-playlists = 还没有本地播放列表
library-no-catalog-songs = 未找到歌曲
library-no-catalog-albums = 未找到专辑
library-no-catalog-artists = 未找到歌手
library-no-matches = 无匹配结果
library-not-loaded = 音乐库加载失败
library-scanning = 正在浏览你的文件夹…
library-scanning-progress = 正在读取你的音乐…{ $read } / { $found }
library-part-not-loaded = 音乐库的这部分加载失败
library-local-unconfigured = 配置本地音乐库

# app menu
app-refresh = 刷新
app-sign-out = 退出登录
app-quit = 退出
app-settings = 设置…
app-hide = 隐藏 Sonora
app-hide-others = 隐藏其他
app-show-all = 显示全部
app-edit = 编辑
app-select-all = 全选
app-window = 窗口
app-close-window = 关闭窗口
app-minimize = 最小化
app-zoom = 缩放

# tray menu
tray-show = 显示 Sonora
tray-play = 播放
tray-pause = 暂停

# table columns
column-played-at = 播放时间
column-index = #
column-title = 标题
column-artist = 歌手
column-album = 专辑
column-genre = 流派
column-date-added = 添加日期
column-added-by = 添加者
column-modified = 修改时间
column-length = 时长
column-plays = 播放次数
column-name = 名称
column-owner = 所有者
column-year = 年份
column-tracks = 歌曲总数

# filters
filter-history = 筛选收听历史
history-empty = 你播放的歌曲会显示在这里。
history-not-loaded = 收听历史加载失败。
history-clear = 清空历史记录
history-clear-title = 清空收听历史
history-clear-confirm = 此设备上的所有播放记录将被移除。此操作无法撤销。
filter-library = 筛选音乐库
filter-album = 筛选专辑歌曲
filter-reset = 重置筛选
filter-duration = 时长
filter-year = 年份
filter-hide-explicit = 隐藏露骨内容
filter-favorites = 仅收藏
filter-downloaded = 已下载
filter-owned = 由你拥有

# view
view-list = 列表
view-cards = 网格

# toolbar
tool-sort = 排序
tool-filters = 筛选

# release kinds
release-album = 专辑
release-single = 单曲
release-compilation = 合辑
release-ep = EP
release-audiobook = 有声书
release-podcast = 播客
kind-song = 歌曲
kind-artist = 歌手
kind-album = 专辑
kind-playlist = 播放列表

# song languages
language-ar = 阿拉伯语
language-de = 德语
language-en = 英语
language-es = 西班牙语
language-fr = 法语
language-hi = 印地语
language-it = 意大利语
language-ja = 日语
language-ko = 韩语
language-pt = 葡萄牙语
language-ru = 俄语
language-tr = 土耳其语
language-uk = 乌克兰语
language-zh = 中文
language-zxx = 无语言内容

# counts
count-songs =
    { $count ->
        [one] { $count } 首歌曲
       *[other] { $count } 首歌曲
    }
count-tracks =
    { $count ->
        [one] { $count } 首歌曲
       *[other] { $count } 首歌曲
    }

# dates
date-just-now = 刚刚
date-minute-ago = 一分钟前
date-minutes-ago = { $count } 分钟前
date-today = 今天 { $time }
date-yesterday = 昨天 { $time }
date-time = { $date }，{ $time }
date-full = { $year }年{ $month }{ $day }日
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

nav-pin = 固定
nav-nothing-pinned = 这里什么都没有
nav-pins-alphabetical = 按字母顺序
nav-pins-kind = 按类型
nav-show-full-library = 显示完整音乐库
nav-return-top = 返回顶部

# power
wake-reason = 正在播放音乐
