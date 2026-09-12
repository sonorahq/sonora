# common
common-on = 开
common-off = 关
common-left = 左
common-right = 右
common-search = 搜索
common-unknown = 未知
common-not-provided = 未提供
common-not-available = 不可用
common-cancel = 取消
common-save = 保存
common-delete = 删除
common-play = 播放
common-more = 更多
common-previous = 上一个
common-next = 下一个
common-dismiss = 忽略
common-clear = 清空
number-group = { "," }

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
library-play-liked-songs = 播放
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
library-part-not-loaded = 音乐库的这部分加载失败
library-local-unconfigured = 配置本地音乐库

# app menu
app-refresh-library = 刷新音乐库
app-sign-out = 退出登录
app-quit = 退出
app-settings = 设置…
app-hide = 隐藏 Sonora
app-hide-others = 隐藏其他
app-show-all = 显示全部
app-edit = 编辑
app-cut = 剪切
app-copy = 复制
app-paste = 粘贴
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
column-date-added = 添加日期
column-added-by = 添加者
column-modified = 修改时间
column-length = 时长
column-plays = 播放次数
column-name = 名称
column-owner = 所有者
column-year = 年份
column-tracks = 歌曲总数

# track menu
menu-add-to-playlist = 添加到播放列表
menu-add-tracks-to-playlist = { $count ->
    [one] 将 { $count } 首歌曲添加到播放列表
   *[other] 将 { $count } 首歌曲添加到播放列表
}
menu-new-playlist = 新建播放列表
menu-edit-tags = 编辑标签
menu-no-playlists = 没有播放列表
menu-add-to-library = 添加到收藏
menu-add-tracks-to-library = { $count ->
    [one] 将 { $count } 首歌曲添加到收藏
   *[other] 将 { $count } 首歌曲添加到收藏
}
menu-remove-from-library = 从收藏中移除
menu-remove-tracks-from-library = { $count ->
    [one] 从收藏中移除 { $count } 首歌曲
   *[other] 从收藏中移除 { $count } 首歌曲
}
menu-remove-from-playlist = 从播放列表中移除
menu-remove-tracks-from-playlist = { $count ->
    [one] 从播放列表中移除 { $count } 首歌曲
   *[other] 从播放列表中移除 { $count } 首歌曲
}
menu-remove-from-history = 从历史记录中移除
menu-remove-tracks-from-history = { $count ->
    [one] 从历史记录中移除 { $count } 首歌曲
   *[other] 从历史记录中移除 { $count } 首歌曲
}
menu-play-next = 下一首播放
menu-play-tracks-next = { $count ->
    [one] 下一首播放 { $count } 首歌曲
   *[other] 下一首播放 { $count } 首歌曲
}
menu-add-to-queue = 添加到播放队列
menu-add-tracks-to-queue = { $count ->
    [one] 将 { $count } 首歌曲添加到播放队列
   *[other] 将 { $count } 首歌曲添加到播放队列
}
menu-song-radio = 前往歌曲电台
menu-go-to-album = 前往专辑
menu-go-to-artist = 前往歌手
menu-view-details = 查看详情
menu-copy-link = 复制链接
menu-cut = 剪切
menu-copy = 复制
menu-paste = 粘贴
menu-select-all = 全选
menu-remove-from-queue = 从播放队列中移除
menu-open-playlist = 打开播放列表
menu-play-playlist = 播放该列表
menu-rename-playlist = 重命名播放列表
menu-delete-playlist = 删除播放列表
menu-add-playlist-to-library = 添加到音乐库
menu-remove-playlist-from-library = 从音乐库中移除
menu-make-playlist-public = 设为公开
menu-make-playlist-private = 设为私密
menu-open-album = 打开专辑
menu-play-album = 播放专辑
menu-play-artist = 播放歌手

# playlist editor
playlist-name-placeholder = 播放列表名称
playlist-create-title = 创建播放列表
playlist-rename-title = 重命名播放列表
playlist-delete-title = 删除播放列表
playlist-delete-confirm = 删除“{ $name }”？此操作无法撤销。
playlist-again-title = 再次添加？
playlist-again-confirm = 这首歌曲已在“{ $name }”中。要再添加一份吗？
playlist-again-add = 再次添加

# confirm
confirm-remove-library-title = 从音乐库中移除
confirm-remove-playlist-title = 从播放列表中移除
confirm-remove-history-title = 从历史记录中移除
confirm-remove-songs = { $count ->
    [one] 从音乐库中移除此歌曲？
   *[other] 从音乐库中移除 { $count } 首歌曲？
}
confirm-remove-playlist-songs = { $count ->
    [one] 从播放列表中移除此歌曲？
   *[other] 从播放列表中移除 { $count } 首歌曲？
}
confirm-remove-history-songs = { $count ->
    [one] 从收听历史中移除此歌曲？
   *[other] 从收听历史中移除 { $count } 首歌曲？
}
confirm-remove-albums = { $count ->
    [one] 从音乐库中移除此专辑？
   *[other] 从音乐库中移除 { $count } 张专辑？
}
confirm-remove-artists = { $count ->
    [one] 从收藏中移除此歌手？
   *[other] 从收藏中移除 { $count } 位歌手？
}
confirm-remove-playlists = { $count ->
    [one] 从音乐库中移除此播放列表？
   *[other] 从音乐库中移除 { $count } 个播放列表？
}

# queue panel
queue-title = 播放队列
queue-history = 历史记录
queue-now-playing = 正在播放
queue-from = 来自
queue-up-next = 接下来播放
queue-reset = 重置
queue-clear = 清空
queue-empty = 播放队列为空
queue-similar = 相似歌曲
queue-radio = 自动播放相似歌曲
queue-return-playing = 返回正在播放

# player bar
player-nothing-playing = 未播放
player-percent = { $value }%
player-shuffle = 随机播放
player-repeat = 循环
player-repeat-all = 全部循环
player-repeat-one = 单曲循环
player-mute = 静音
player-unmute = 取消静音
player-previous = 上一首
player-next = 下一首
player-fullscreen = 全屏
player-fullscreen-leave = 退出全屏
fullscreen-artwork = 封面

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
filter-explicit = 仅露骨内容
filter-playable = 仅播放
filter-favorites = 仅收藏
filter-owned = 由你拥有

# view
view-list = 列表
view-cards = 网格

# toolbar
tool-columns = 列
tool-sort = 排序
tool-filters = 筛选

# login
login-signed-out = 登录以加载你的音乐库
login-restoring = 正在检查已保存的会话…
login-authorizing = 正在浏览器中等待授权…
login-signed-in = 已登录为 { $name }
login-failed-title = 登录失败
login-problem-region = Spotify 不会从你所在的国家/地区开启会话。请从你的所在国家/地区登录，或更改 Spotify 账户上的国家/地区。
login-problem-credentials = 你保存的 Spotify 会话已失效。请重新登录以继续。
login-problem-network = Sonora 无法连接到 Spotify。请检查网络连接并重试。
login-problem-cancelled = 你在批准登录之前关闭了浏览器页面。请重新开始以完成登录。
login-problem-refused = Spotify 拒绝了登录请求。请稍等片刻后重试。
login-problem-premium = Sonora 通过 Spotify Premium 串流，而此账户没有 Premium。请使用 Premium 账户登录以继续。
login-sign-in = 使用 { $provider } 登录
login-connect-cookies = 手动粘贴 Cookies
login-cookie-open = 打开 YouTube Music
login-cookie-submit = 继续
login-cookie-hint = 在此粘贴 Cookie 请求头
login-cookie-step-1 = 打开 music.youtube.com 并确保你已登录。使用无痕窗口效果最好。
login-cookie-step-2 = 按 F12，打开网络（Network）标签页并刷新页面。
login-cookie-step-3 = 选择任意名为“browse”或“next”的请求。
login-cookie-step-4 = 在标头（Headers）中，找到请求标头（Request Headers）下的 Cookie，右键点击并复制其值。
login-cookie-step-note = 请确保粘贴完整值，包括 SAPISID 和 __Secure-3PAPISID。
login-cookie-title = 粘贴你的 YouTube Music cookies 以完成登录
login-window-title = 使用 { $provider } 登录
login-use = 使用 { $provider }
login-guest-title = 访客模式
login-guest-use = 使用访客模式
login-guest-detail = 无需账户即可浏览和播放。你的音乐库、收藏和播放列表将无法访问。
login-usage-consent = 帮助我们估算有多少人使用 Sonora。
login-device-code = 在 { $url } 输入此代码
login-server-title = 连接到你的 Subsonic 服务器
login-server-detail = 输入任意 Subsonic 或 OpenSubsonic 服务器（Navidrome、Airsonic、Gonic……）的地址，然后使用服务器用户名和密码登录。会话会保留在此设备上。
login-server-hint = https://music.example.com
login-username-hint = 用户名
login-password-hint = 密码
login-server-submit = 连接
login-account-title = 选择一个账户
login-account-detail = 此会话登录了多个 Google 账户。请选择 Sonora 应使用的账户。

# album and playlist pages
detail-album = 专辑
detail-playlist = 播放列表
detail-play-album = 播放专辑
detail-play-playlist = 播放该列表

# play button
play-pause = 暂停
play-resume = 继续播放
play-loading = 加载中…
play-shuffle = 随机播放

# artist page
artist-eyebrow = 歌手
artist-monthly-listeners = { $count ->
    [one] { $value } 位每月听众
   *[other] { $value } 位每月听众
}
artist-play = 立即播放
artist-popular = 热门
artist-popular-eyebrow = 探索这位歌手
artist-popular-empty = 这位歌手暂无可播放内容
artist-popular-more = 显示全部
artist-popular-less = 收起
artist-releases = 发行作品
artist-releases-more = 显示全部
artist-releases-less = 收起
artist-filter-all = 全部
artist-filter-albums = 专辑
artist-filter-singles = 单曲
artist-filter-eps = EP

# user profile page
user-eyebrow = 个人资料
user-followers = { $count ->
    [one] { $value } 位关注者
   *[other] { $value } 位关注者
}
user-following = { $count ->
   *[other] 正在关注 { $value } 人
}
user-playlists = 公开播放列表
user-playlists-empty = 还没有公开播放列表

# release kinds
release-album = 专辑
release-single = 单曲
release-compilation = 合辑
release-ep = EP
release-audiobook = 有声书
release-podcast = 播客
release-meta = { $year } • { $kind }

# home page
home-quick-picks = 快速推荐
home-listen-again = 再次收听
home-quick-picks-eyebrow = 从一首歌开始
home-quick-picks-empty = 收藏几首歌后，它们会显示在这里

# search page
search-placeholder = 你想听什么？
search-browse = 浏览全部
genre-empty = 这里暂时没有可显示的内容
search-best-match = 最佳匹配
search-no-matches = 无匹配结果
search-results = 结果
search-songs = 歌曲
search-artists = 歌手
search-albums-playlists = 专辑和播放列表
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] 音乐库中有 { $count } 首歌曲
       *[other] 音乐库中有 { $count } 首歌曲
    }
kind-song = 歌曲
kind-artist = 歌手
kind-album = 专辑
kind-playlist = 播放列表

# song page
song-eyebrow = 歌曲
song-play = 播放歌曲
song-view-album = 查看专辑
song-loading = 正在加载歌曲信息…
song-about = 关于这首歌
song-album = 专辑
song-released = 发行日期
song-streams = 播放次数
song-position = 位置
song-label = 唱片公司
song-popularity = 热度
song-popularity-value = { $value }%
song-disc-track = 第 { $disc } 张碟，第 { $track } 首
song-track = 第 { $track } 首
song-credits = 制作人员
song-performed-by = 表演者
song-details = 流派和详情
song-genres = 流派
song-language = 语言
song-content = 内容
song-explicit = 露骨内容
song-clean = 干净版
artist-about = 关于歌手
artist-about-fallback = 探索这位歌手的热门歌曲和发行作品。
artist-about-open = 前往歌手
song-copyright = © { $notice }

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

# settings
settings-tab-general = 通用
settings-tab-appearance = 外观
settings-tab-playback = 播放
settings-tab-privacy = 隐私
settings-tab-integrations = 集成
settings-theme = 主题
settings-theme-detail = 选择应用的配色方案
settings-opacity = 不透明度
settings-opacity-detail = 调整应用背景的不透明度
settings-opacity-value = { $percent }%
settings-theme-config = 打开配置
settings-adaptive = 自适应主题
settings-adaptive-detail = 使用正在播放专辑的封面为配色方案着色
settings-visualizer = 可视化效果
settings-visualizer-detail = 在全屏封面后显示频谱条
settings-icons = 图标包
settings-icons-detail = 选择界面使用的图标集
settings-motion = 减少动画
settings-motion-detail = 跳过界面动画和过渡效果
settings-pace = 动画速度
settings-pace-detail = 界面动画的播放速度
settings-saver = 省电模式
settings-saver-detail = 当 Sonora 未聚焦时限制动画帧率，从下次启动起生效
settings-corners = 圆角
settings-corners-detail = 界面和控件的圆角程度
settings-blur = 模糊
settings-blur-detail = 在模糊的桌面上绘制窗口。需要不透明度低于 100%
settings-font = 字体大小
settings-font-detail = 基础文字大小，其他所有内容随之缩放
settings-font-value = { $size } px
settings-startup = 欢迎页
settings-startup-detail = Sonora 启动时打开的页面
settings-entries = 侧边栏项
settings-entries-detail = 侧边栏中显示的项
settings-entries-pick = 选择项
settings-language = 语言
settings-language-detail = Sonora 在整个界面中使用的语言
settings-language-system = 系统
settings-language-search = 搜索语言
settings-language-none = 未找到语言
settings-typeface = 字体
settings-typeface-detail = Sonora 在整个界面中使用的字体
settings-typeface-system = 默认
settings-typeface-search = 搜索字体
settings-typeface-none = 未找到字体
settings-server-side-decorations = 服务端窗口装饰
settings-server-side-decorations-detail = 让合成器绘制标题栏、边框和阴影
settings-typeface-loading = 加载中…
settings-window-controls = 窗口控件
settings-window-controls-detail = 在标题栏中绘制最小化、最大化和关闭按钮
settings-traffic-light-controls = 红绿灯按钮
settings-traffic-light-controls-detail = 将最小化、最大化和关闭绘制为彩色圆点
settings-window-rounding = 窗口圆角
settings-window-rounding-detail = 窗口自身角落的圆角程度
settings-controls-side = 控件位置
settings-controls-side-detail = 控件位于标题栏的哪一端
settings-close-to-tray = 关闭主窗口
settings-close-to-tray-detail = 窗口关闭后将 Sonora 保留在系统托盘中并继续播放
settings-discord = 在 Discord 上显示
settings-discord-detail = 将你正在播放的歌曲显示在 Discord 个人资料上
settings-discord-name = 状态名称
settings-discord-name-detail = 状态在“正在收听”后显示的名称，你的好友会看到
settings-discord-name-sonora = Sonora
settings-discord-name-provider = 服务提供商
settings-discord-name-music = 音乐
settings-discord-name-title = 标题
settings-discord-name-artist = 歌手
settings-discord-name-artist-title = 歌手 - 标题
settings-discord-show-paused = 暂停时显示
settings-discord-show-paused-detail = 歌曲暂停时仍在 Discord 个人资料上保留状态
settings-discord-badge = 显示服务提供商徽章
settings-discord-badge-detail = 用歌曲来源服务的小图标标记状态
settings-discord-anonymous = 隐藏详细信息
settings-discord-anonymous-detail = 仅显示正在播放音乐，不显示标题、歌手或封面
# the Discord status when the track is left out of it
discord-listening = 正在收听音乐
settings-normalisation = 系统音量
settings-normalisation-detail = 保持歌曲音量一致
settings-gapless = 无缝播放
settings-gapless-detail = 歌曲之间无间隙地连续播放，如同专辑原本的编排
settings-sleep = 睡眠定时器
settings-sleep-detail = 让音乐在设定时间后自动停止，伴你入眠
settings-sleep-configure = 配置…
settings-sleep-off = 关闭
settings-sleep-end-of-track = 歌曲结束后
settings-sleep-minutes = { $count } 分钟
settings-panel-lyrics-size = 歌词大小（面板）
settings-panel-lyrics-size-detail = 侧边面板中歌词文字的大小，基于基础字体大小
settings-fullscreen-lyrics-size = 歌词大小（全屏）
settings-fullscreen-lyrics-size-detail = 全屏播放器中歌词文字的大小，基于基础字体大小
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = 本地文件歌词
settings-lyrics-for-local-files-detail = 使用本地文件的元数据从互联网获取歌词
settings-karaoke-lyrics = 逐字歌词
settings-karaoke-lyrics-detail = 歌词有时间轴时，逐字高亮显示
settings-blur-lyrics = 模糊非当前歌词
settings-blur-lyrics-detail = 在歌词面板中模糊尚未播放和已经播放的歌词行
settings-romanized-lyrics = 罗马化歌词
settings-romanized-lyrics-detail = 为选定的书写系统显示本地生成的发音
settings-romanization-writing-systems = 书写系统
settings-romanization-japanese = 日语
settings-romanization-chinese = 中文
settings-romanization-korean = 韩语
settings-romanization-cyrillic = 西里尔字母
settings-romanization-greek = 希腊语
settings-romanization-arabic = 阿拉伯语
settings-romanization-other = 其他书写系统
settings-advanced = 高级
settings-group-window = 窗口
settings-group-accounts = 账户
settings-group-library = 音乐库
settings-group-text = 文字
settings-group-motion = 动画
settings-group-title-bar = 标题栏
settings-group-window-style = 窗口样式
settings-group-lyrics = 歌词
settings-group-discord = Discord
settings-group-project = 项目
settings-adaptive-menu = 自适应上下文菜单
settings-adaptive-menu-detail = 省略行中已显示的项，例如专辑或歌手
settings-accounts = 管理账户
settings-accounts-detail = 此设备可以播放的服务
settings-provider-none = 未连接
settings-provider-connected = 已连接
settings-provider-current = 正在从此服务播放
settings-provider-guest = 以访客身份播放
settings-provider-switch = 切换到
settings-sign-out = 退出登录
settings-local-folder = 音乐文件夹
settings-local-folder-empty = 未配置
settings-choose-folder = 选择文件夹…
settings-add-folder = 添加文件夹
settings-remove-folder = 移除文件夹
settings-rescan = 重新扫描
settings-tab-about = 关于
settings-version = 版本
settings-version-detail = 你正在运行的 Sonora 版本
settings-license = 许可证
settings-license-detail = GNU 通用公共许可证第 3 版或更高版本
settings-license-view = 阅读许可证
settings-source = 源代码
settings-source-detail = 此构建对应的源代码
settings-source-view = 打开仓库
settings-team = 团队
settings-team-github = GitHub
settings-role-lead-maintainer = 首席维护者
settings-role-maintainer = 维护者
settings-role-contributor = 贡献者
settings-notice = 版权所有 © 2026 Sonora 贡献者。Sonora 不提供任何担保。它是自由软件，你可以根据 GNU 通用公共许可证第 3 版或更高版本的条款重新分发。Sonora 是非官方项目，与 Spotify AB 无关联。

# themes
theme-system = 系统
theme-dark = 深色
theme-light = 浅色
theme-midnight = 午夜
theme-forest = 森林
theme-ocean = 海洋
theme-rose = 玫瑰
theme-lavender = 薰衣草
theme-amber = 琥珀

# corners
corners-square = 直角
corners-subtle = 微圆
corners-rounded = 圆角
corners-round = 全圆

# motion
motion-system = 系统
motion-always = 始终
motion-never = 从不
pace-slow = 慢
pace-base = 标准
pace-quick = 快
saver-off = 关
saver-light = 轻度（{ $fps } FPS）
saver-medium = 中度（{ $fps } FPS）
saver-strong = 强力（{ $fps } FPS）

toast-playlist-created = 播放列表已创建
toast-playlist-renamed = 播放列表已重命名
toast-playlist-deleted = 播放列表已删除
toast-playlist-added = 播放列表已添加到你的音乐库
toast-playlist-removed = 播放列表已从你的音乐库中移除
toast-playlist-visibility = 播放列表可见性已更改
toast-track-added = 已添加到 { $name }
toast-track-removed = 已从 { $name } 中移除
toast-playlist-failed = 该更改无法保存
toast-playlist-busy = 另一个更改仍在进行中
toast-playlist-signed-out = 登录以更改播放列表
toast-queued-track = { $name } 已添加到播放队列
toast-next-track = 接下来播放 { $name }
toast-queued-album = 专辑已添加到播放队列
toast-next-album = 接下来播放专辑
toast-queued-playlist = 播放列表已添加到播放队列
toast-next-playlist = 接下来播放该列表
toast-queued-artist = 歌手已添加到播放队列
toast-next-artist = 接下来播放歌手
toast-queue-failed = 无法添加到播放队列
toast-keys-refused = Spotify 未授予此账户播放密钥
toast-sign-in-to-play = { $name } 仅向已登录用户播放
toast-track-unplayable = { $name } 无法播放
toast-library-add-failed = { $name } 无法添加到你的音乐库
toast-library-remove-failed = { $name } 无法从你的音乐库中移除

# lyrics
lyrics-title = 歌词
lyrics-idle = 播放一些内容以查看歌词
lyrics-loading = 正在查找歌词…
lyrics-missing = 抱歉，未找到歌词！
lyrics-instrumental = 这首歌是纯音乐
lyrics-failed = 无法连接到歌词服务
lyrics-follow = 重新跟随歌曲
lyrics-source = 歌词来自 { $source }
lyrics-writers = 由 { $writers } 创作

update-available = Sonora { $version } 已发布
update-detail = 你当前使用的是 { $running }。查看更新内容，或立即更新。
update-detail-notes = 你当前使用的是 { $running }。查看更新内容，然后按你的安装方式更新 Sonora。
update-notes = 新内容
update-now = 更新
update-later = 稍后
update-working = 正在下载更新…
update-failed = 更新无法安装。请从发布页面重试。
settings-check-updates = 检查更新
settings-check-updates-detail = 启动时向 GitHub 查询一次是否有新版本。Sonora 仅在 Windows 上自行安装更新；其他平台会指引你查看更新内容

# tags
tags-edit-title = 编辑标签
tags-sheet-song = 歌曲
tags-sheet-album = 专辑
tags-sheet-details = 详情
tags-title = 标题
tags-artist = 歌手
tags-track = 歌曲编号
tags-track-total = 发行歌曲总数
tags-disc = 碟片编号
tags-disc-total = 发行碟片总数
tags-album = 专辑
tags-album-artist = 专辑歌手
tags-year = 年份
tags-genre = 流派
tags-composer = 作曲家
tags-publisher = 发行商
tags-isrc = ISRC
tags-comment = 备注
toast-tags-saved = 已保存 { $name } 的标签
toast-tags-failed = 标签无法保存

nav-pin = 固定
toast-library-pin-limit = 已达到 Spotify 的固定上限。请先取消固定其他项目。
toast-library-pin-failed = 无法在 Spotify 中更新固定状态。
nav-nothing-pinned = 这里什么都没有
nav-pins-alphabetical = 按字母顺序
nav-pins-kind = 按类型
nav-show-full-library = 显示完整音乐库
nav-return-top = 返回顶部
