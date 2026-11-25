# common
common-left = 左
common-right = 右
common-search = 検索
common-unknown = 不明
common-not-provided = 情報なし
common-not-available = データなし
common-unavailable = { $name }（利用不可）
common-cancel = キャンセル
common-save = 保存
common-delete = 削除
common-play = 再生
common-more = その他
common-previous = 前へ
common-next = 次へ
common-dismiss = 閉じる
common-clear = クリア
common-undo = 元に戻す
common-redo = やり直す
common-cut = カット
common-copy = コピー
common-paste = ペースト
common-show-all = すべて表示
common-show-less = 折りたたむ
common-local-files = ローカルファイル
common-server-address = サーバーアドレス
common-api-key = API キー
common-not-now = 後で
common-not-connected = 未接続
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
runtime-seconds = { $seconds }秒
runtime-minutes = { $minutes }分{ $seconds }秒
runtime-hours = { $hours }時間{ $minutes }分

# navigation
nav-history = 履歴
nav-home = ホーム
nav-search = 検索
nav-library = ライブラリ
nav-settings = 設定
nav-songs = 曲
nav-albums = アルバム
nav-playlists = プレイリスト
nav-artists = アーティスト
nav-local = ローカル音楽
nav-back = 戻る
nav-forward = 進む
nav-sidebar = サイドバーを切り替え
nav-sidebar-right = 歌詞と再生キューを表示・非表示
nav-pinned = ピン留め
nav-unpin = ピン留めを解除
nav-pin-hint = ここにドロップしてピン留め
library-liked-songs = お気に入り
library-no-songs = お気に入りはまだありません
library-no-albums = 保存したアルバムはまだありません
library-no-playlists = プレイリストはまだありません
library-no-artists = お気に入りのアーティストはまだいません
library-no-local-songs = インポートされた曲が見つかりません
library-no-local-albums = インポートされたアルバムが見つかりません
library-no-local-artists = インポートされたアーティストが見つかりません
library-no-local-playlists = ローカルのプレイリストはまだありません
library-no-catalog-songs = 曲が見つかりません
library-no-catalog-albums = アルバムが見つかりません
library-no-catalog-artists = アーティストが見つかりません
library-no-matches = 一致する項目がありません
library-not-loaded = ライブラリを読み込めませんでした
library-scanning = フォルダを確認しています…
library-scanning-progress = 音楽を読み込んでいます… { $read } / { $found }
library-part-not-loaded = ライブラリのこの部分を読み込めませんでした
library-local-unconfigured = ローカルライブラリを設定する

# app menu
app-refresh = 再読み込み
app-sign-out = サインアウト
app-quit = 終了
app-settings = 設定…
app-hide = Sonoraを隠す
app-hide-others = ほかを隠す
app-show-all = すべてを表示
app-edit = 編集
app-select-all = すべてを選択
app-window = ウインドウ
app-close-window = ウインドウを閉じる
app-minimize = しまう
app-zoom = 拡大/縮小

# tray menu
tray-show = Sonora を表示
tray-play = 再生
tray-pause = 一時停止

# table columns
column-played-at = 再生日時
column-index = #
column-title = タイトル
column-artist = アーティスト
column-album = アルバム
column-genre = ジャンル
column-date-added = 追加日
column-added-by = 追加者
column-modified = 更新日
column-length = 長さ
column-plays = 再生数
column-name = 名前
column-owner = 所有者
column-year = 年
column-tracks = 曲数

# filters
filter-history = 再生履歴を絞り込む
history-empty = 再生した曲がここに表示されます。
history-not-loaded = 再生履歴を読み込めませんでした。
history-clear = 履歴をクリア
history-clear-title = 再生履歴をクリア
history-clear-confirm = このデバイスからすべての再生記録が削除されます。この操作は取り消せません。
filter-library = ライブラリを絞り込む
filter-album = アルバムの曲を絞り込む
filter-reset = フィルターをリセット
filter-duration = 長さ
filter-year = 年
filter-hide-explicit = 露骨な表現を含む曲を隠す
filter-favorites = お気に入りのみ
filter-downloaded = ダウンロード済み
filter-owned = 自分のもの

# view
view-list = リスト
view-cards = グリッド

# toolbar
tool-sort = 並べ替え
tool-filters = フィルター

# release kinds
release-album = アルバム
release-single = シングル
release-compilation = コンピレーション
release-ep = EP
release-audiobook = オーディオブック
release-podcast = ポッドキャスト
kind-song = 曲
kind-artist = アーティスト
kind-album = アルバム
kind-playlist = プレイリスト

# song languages
language-ar = アラビア語
language-de = ドイツ語
language-en = 英語
language-es = スペイン語
language-fr = フランス語
language-hi = ヒンディー語
language-it = イタリア語
language-ja = 日本語
language-ko = 韓国語
language-pt = ポルトガル語
language-ru = ロシア語
language-tr = トルコ語
language-uk = ウクライナ語
language-zh = 中国語
language-zxx = 歌詞なし

# counts
count-songs =
    { $count ->
       *[other] { $count }曲
    }
count-tracks =
    { $count ->
       *[other] { $count }曲
    }

# dates
date-just-now = たった今
date-minute-ago = 1分前
date-minutes-ago = { $count }分前
date-today = 今日 { $time }
date-yesterday = 昨日 { $time }
date-time = { $date } { $time }
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

nav-pin = ピン留め
nav-nothing-pinned = ここには何もありません
nav-pins-alphabetical = 名前順
nav-pins-kind = 種類別
nav-show-full-library = ライブラリ全体を表示
nav-return-top = 先頭に戻る

# power
wake-reason = 音楽を再生中
