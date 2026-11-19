# track menu
menu-new-playlist = 新建播放列表
menu-import-playlist = 导入播放列表…
menu-no-playlists = 没有播放列表
menu-search-playlists = 搜索播放列表
menu-add-to-library = 添加到收藏
menu-remove-from-library = 从收藏中移除
menu-remove-tracks-from-playlist = { $count ->
    [one] 从播放列表中移除 { $count } 首歌曲
   *[other] 从播放列表中移除 { $count } 首歌曲
}
menu-remove-tracks-from-history = { $count ->
    [one] 从历史记录中移除 { $count } 首歌曲
   *[other] 从历史记录中移除 { $count } 首歌曲
}
menu-download = 下载
menu-remove-download = 移除下载
menu-downloading = 正在下载。点击停止
menu-song-radio = 前往歌曲电台
menu-go-to-album = 前往专辑
menu-go-to-artist = 前往歌手
menu-view-details = 查看详情
menu-copy-link = 复制链接
menu-select-all = 全选
menu-remove-from-queue = 从播放队列中移除
menu-export-playlist = 导出播放列表…
menu-add-playlist-to-library = 添加到音乐库
menu-remove-playlist-from-library = 从音乐库中移除
menu-make-playlist-public = 设为公开
menu-make-playlist-private = 设为私密
menu-play-artist = 播放歌手

# playlist editor
menu-copy-link-none = 此项没有可分享的链接
menu-copy-link-local = 本地歌手和播放列表没有链接
menu-play-next = { $count ->
    [1] 下一首播放
    [one] 下一首播放 { $count } 首歌曲
   *[other] 下一首播放 { $count } 首歌曲
}
menu-add-to-queue = { $count ->
    [1] 添加到播放队列
    [one] 将 { $count } 首歌曲添加到播放队列
   *[other] 将 { $count } 首歌曲添加到播放队列
}
menu-play-last = { $count ->
    [1] 最后播放
    [one] 最后播放 { $count } 首歌曲
   *[other] 最后播放 { $count } 首歌曲
}
menu-delete-track-files = { $count ->
    [1] 删除歌曲文件
   *[other] 删除 { $count } 个歌曲文件
}

# playlist editor
playlist-name-placeholder = 播放列表名称
playlist-create-title = 创建播放列表
playlist-edit-title = 编辑播放列表
playlist-cover-choose = 选择图片
playlist-cover-remove = 移除封面
playlist-cover-unopenable = Sonora 无法打开此文件
playlist-cover-too-large = 这张图片超过 40 MB
playlist-cover-not-a-picture = 此文件不是 JPEG、PNG 或 WebP 图片
playlist-delete-title = 删除播放列表
playlist-again-title = 再次添加？
playlist-again-confirm = 这首歌曲已在“{ $name }”中。要再添加一份吗？
playlist-again-add = 再次添加

# confirm

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
confirm-delete-track-files-title = 删除歌曲文件？
confirm-delete-track-files = 文件将从磁盘上永久删除。此操作无法撤销。
confirm-uninstall-widevine-title = 卸载 Widevine 模块？
confirm-uninstall-widevine = Sonora 将删除从 Google 下载的副本。在重新下载之前，Apple Music 的歌曲将无法播放。
confirm-remove-downloads-title = 移除所有下载？
confirm-remove-downloads = 你从所有服务下载的歌曲都将从此电脑上删除。在重新下载之前，它们将改为在线串流。
confirm-delete-named = 删除“{ $name }”？此操作无法撤销。
