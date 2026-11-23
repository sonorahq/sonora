members-title = Thành viên
members-empty = Bắt đầu hoặc tham gia một phiên nghe để xem ai đang nghe cùng bạn

# login
login-restoring = Đang kiểm tra phiên đã lưu…
login-authorizing = Đang chờ cấp quyền trong trình duyệt…
login-signed-in = Đã đăng nhập với tên { $name }
login-failed-title = Đăng nhập thất bại
login-problem-region = Spotify không mở phiên từ quốc gia bạn đang ở. Hãy đăng nhập từ quốc gia của bạn, hoặc đổi quốc gia trong tài khoản Spotify.
login-problem-credentials = Phiên đã lưu cho dịch vụ này không còn hợp lệ. Hãy đăng nhập lại để tiếp tục.
login-problem-network = Sonora không kết nối được với dịch vụ nhạc. Hãy kiểm tra kết nối internet và thử lại.
login-problem-cancelled = Yêu cầu đăng nhập chưa được chấp thuận trong trình duyệt hoặc đã hết thời gian. Hãy bắt đầu lại để hoàn tất.
login-problem-refused = Spotify đã từ chối đăng nhập. Hãy chờ một lát rồi thử lại.
login-problem-premium = Sonora phát nhạc qua Spotify Premium, nhưng tài khoản này không có Premium. Hãy đăng nhập bằng tài khoản Premium để tiếp tục.
login-problem-subscription = Apple ID này không có gói đăng ký nào đang hoạt động.
login-subscription-check = Kiểm tra Apple Music
login-sign-in = Đăng nhập bằng { $provider }
login-connect-cookies = Dán cookie thủ công
login-cookie-open = Mở { $provider }
login-cookie-submit = Tiếp tục
login-cookie-hint = Dán header yêu cầu Cookie vào đây
login-cookie-step-1 = Mở music.youtube.com và đảm bảo bạn đã đăng nhập. Tốt nhất nên dùng cửa sổ ẩn danh.
login-cookie-step-2 = Nhấn F12, mở tab Network và tải lại trang.
login-cookie-step-3 = Chọn một yêu cầu bất kỳ có tên "browse" hoặc "next".
login-cookie-step-4 = Trong Headers, tìm Cookie trong mục Request Headers, nhấp chuột phải và sao chép giá trị.
login-cookie-step-note = Hãy dán toàn bộ giá trị, bao gồm cả SAPISID và __Secure-3PAPISID.
login-cookie-header-title = Dán cookie { $provider } để hoàn tất đăng nhập
login-cookie-apple-step-3 = Chọn một yêu cầu bất kỳ đến { $site }.
login-cookie-apple-note = Hãy dán toàn bộ giá trị, bao gồm cả { $cookie }.
login-cookie-named-title = Dán cookie { $provider } để hoàn tất đăng nhập
login-cookie-named-hint = Dán giá trị cookie vào đây
login-cookie-named-step-1 = Mở { $site } và đảm bảo bạn đã đăng nhập.
login-cookie-named-step-2 = Nhấn F12 và mở tab Storage trong Firefox, hoặc tab Application trong Chrome.
login-cookie-named-step-3 = Mở rộng Cookies, chọn { $site } và tìm cookie có tên { $cookie }.
login-cookie-named-step-4 = Nhấp đúp vào giá trị của cookie và sao chép.
login-cookie-named-note = Chỉ cần giá trị { $cookie } là đủ. Toàn bộ header Cookie sao chép từ tab Network cũng dùng được.
login-guest-title = Chế độ khách
login-guest-use = Dùng chế độ khách
login-usage-consent = Giúp chúng tôi ước tính số người dùng Sonora.
login-device-code = Nhập mã này tại { $url }
login-server-title = Kết nối với máy chủ Subsonic của bạn
login-server-detail = Nhập địa chỉ của bất kỳ máy chủ Subsonic hoặc OpenSubsonic nào (Navidrome, Airsonic, Gonic, …), sau đó đăng nhập bằng tên người dùng và mật khẩu trên máy chủ. Phiên được lưu trên thiết bị này.
login-server-hint = https://music.example.com
login-username-hint = Tên người dùng
login-password-hint = Mật khẩu
login-server-submit = Kết nối
login-account-title = Chọn tài khoản
login-account-detail = Phiên này đã đăng nhập vào nhiều tài khoản Google. Hãy chọn tài khoản Sonora sẽ dùng.
login-choose-title = Đăng nhập vào { $provider }
login-choose-detail = Chọn cách đăng nhập vào { $provider }.
login-providers = Dịch vụ nhạc

# album and playlist pages

# artist page
artist-eyebrow = Nghệ sĩ
artist-monthly-listeners = { $count ->
   *[other] { $value } người nghe hằng tháng
}
artist-followers = { $count ->
   *[other] { $value } người theo dõi
}
artist-popular = Phổ biến
artist-popular-eyebrow = Khám phá nghệ sĩ này
artist-popular-empty = Chưa có gì để phát từ nghệ sĩ này
artist-favorite-songs = Bài hát yêu thích của bạn
artist-favorite-songs-count = { $count ->
   *[other] { $count } bài hát trong Yêu thích
}
artist-releases = Bản phát hành
artist-filter-all = Tất cả
artist-filter-singles = Đĩa đơn
artist-filter-eps = EP
artist-appears-on = Xuất hiện trong
album-also-like = Có thể bạn cũng thích
album-tab-artists = Nghệ sĩ
album-label = ℗ { $label }

# user profile page
user-eyebrow = Hồ sơ
user-followers = { $count ->
   *[other] { $value } người theo dõi
}
user-following = { $count ->
   *[other] Đang theo dõi { $value }
}
user-playlists = Danh sách phát công khai
user-playlists-empty = Chưa có danh sách phát công khai nào

# home page
home-quick-picks = Lựa chọn nhanh
home-quick-picks-empty = Hãy thích vài bài hát và chúng sẽ xuất hiện ở đây
home-recently-added = Mới thêm gần đây
home-recently-played = Mới phát gần đây
home-favorite-albums = Album yêu thích
home-artists = Nghệ sĩ
home-collection-albums = Album từ bộ sưu tập của bạn

# powerbar (quick search)
powerbar-title = Tìm kiếm nhanh
powerbar-open = Mở
powerbar-song = Chuyển đến bài hát

# search page
search-placeholder = Bạn muốn nghe gì?
search-browse = Duyệt tìm tất cả
search-submit = Tìm kiếm
search-open-link = Mở liên kết trong Sonora
search-recent = Tìm kiếm gần đây
search-forget = Xóa khỏi lịch sử
genre-empty = Chưa có gì để hiển thị ở đây
search-best-match = Kết quả phù hợp nhất
search-no-matches = Không có kết quả phù hợp
search-results = Kết quả
search-artists = Nghệ sĩ
search-albums-playlists = Album và danh sách phát
search-tag = { $kind } ·
search-saved =
    { $count ->
       *[other] { $count } bài hát trong Thư viện
    }

# song page
song-eyebrow = Bài hát
song-view-album = Xem album
song-about = Giới thiệu bài hát
song-album = Album
song-released = Phát hành
song-streams = Lượt phát
song-position = Vị trí
song-label = Hãng đĩa
song-popularity = Mức độ phổ biến
song-disc-track = Đĩa { $disc }, bài { $track }
song-track = Bài { $track }
song-credits = Ghi công
song-performed-by = Trình bày bởi
song-details = Thể loại và chi tiết
song-genres = Thể loại
song-language = Ngôn ngữ
song-content = Nội dung
song-explicit = Tục tĩu
song-local = Tệp cục bộ
song-clean = Không tục tĩu
artist-about = Giới thiệu nghệ sĩ
artist-about-fallback = Khám phá các bài hát và bản phát hành nổi bật của nghệ sĩ.
artist-about-open = Chuyển đến nghệ sĩ
song-copyright = © { $notice }
track-row-name = { $title } của { $artists }

# tags
tags-edit-title = Sửa thẻ
tags-sheet-song = Bài hát
tags-sheet-album = Album
tags-sheet-details = Chi tiết
tags-title = Tiêu đề
tags-artist = Nghệ sĩ
tags-artists = Các nghệ sĩ
tags-add-artist = Thêm nghệ sĩ
tags-remove-artist = Xóa nghệ sĩ này
tags-track = Số thứ tự bài hát
tags-track-total = Số bài hát trong bản phát hành
tags-disc = Số đĩa
tags-disc-total = Tổng số đĩa trong bản phát hành
tags-album = Album
tags-album-artist = Nghệ sĩ album
tags-year = Năm
tags-genre = Thể loại
tags-composer = Nhà soạn nhạc
tags-publisher = Nhà phát hành
tags-isrc = ISRC
tags-comment = Ghi chú
