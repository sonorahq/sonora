members-title = 멤버
members-empty = 함께 듣는 사람을 보려면 세션을 시작하거나 참여하세요.

# login
login-restoring = 저장된 세션 확인 중…
login-authorizing = 브라우저에서 인증을 기다리는 중…
login-signed-in = { $name }(으)로 로그인됨
login-failed-title = 로그인 실패
login-problem-region = 현재 계신 국가에서는 Spotify 세션을 열 수 없습니다. 거주 국가에서 로그인하거나 Spotify 계정의 국가 설정을 변경하세요.
login-problem-credentials = 이 서비스에 저장된 세션이 더 이상 유효하지 않습니다. 계속하려면 다시 로그인하세요.
login-problem-password = 서버에서 해당 사용자 이름과 비밀번호를 허용하지 않았습니다. 확인 후 다시 시도하세요.
login-problem-network = Sonora가 음악 서비스에 연결할 수 없습니다. 인터넷 연결을 확인하고 다시 시도하세요.
login-problem-cancelled = 브라우저에서 로그인이 승인되지 않았거나 시간이 초과되었습니다. 완료하려면 처음부터 다시 시작하세요.
login-problem-refused = Spotify가 로그인을 거부했습니다. 잠시 후 다시 시도하세요.
login-problem-premium = Sonora는 Spotify Premium을 통해 스트리밍되지만, 이 계정에는 해당 멤버십이 없습니다. 계속하려면 Premium 계정으로 로그인하세요.
login-problem-subscription = 이 Apple ID에 활성화된 구독이 없습니다.
login-subscription-check = Apple Music 확인
login-sign-in = { $provider }(으)로 로그인
login-connect-cookies = 쿠키 수동 붙여넣기
login-cookie-open = { $provider } 열기
login-cookie-submit = 계속
login-cookie-hint = 여기에 Cookie 요청 헤더를 붙여넣으세요.
login-cookie-step-1 = music.youtube.com을 열고 로그인되어 있는지 확인하세요. 시크릿 창을 사용하는 것이 가장 좋습니다.
login-cookie-step-2 = F12 키를 누르고 네트워크(Network) 탭을 연 다음 페이지를 새로고침하세요.
login-cookie-step-3 = 이름이 "browse" 또는 "next"인 요청을 선택하세요.
login-cookie-step-4 = 헤더(Headers)에서 요청 헤더(Request Headers) 아래의 Cookie를 찾아 마우스 오른쪽 버튼으로 클릭하고 값을 복사하세요.
login-cookie-step-note = SAPISID 및 __Secure-3PAPISID를 포함한 전체 값을 붙여넣어야 합니다.
login-cookie-header-title = 로그인을 완료하려면 { $provider } 쿠키를 붙여넣으세요.
login-cookie-apple-step-3 = { $site }에 대한 아무 요청이나 선택하세요.
login-cookie-apple-note = { $cookie }을(를) 포함한 전체 값을 붙여넣어야 합니다.
login-cookie-named-title = 로그인을 완료하려면 { $provider } 쿠키를 붙여넣으세요.
login-cookie-named-hint = 여기에 쿠키 값을 붙여넣으세요.
login-cookie-named-step-1 = { $site }을(를) 열고 로그인되어 있는지 확인하세요.
login-cookie-named-step-2 = F12 키를 누르고 Firefox에서는 저장소(Storage) 탭을, Chrome에서는 애플리케이션(Application) 탭을 여세요.
login-cookie-named-step-3 = 쿠키(Cookies)를 확장하고 { $site }을(를) 선택한 다음 이름이 { $cookie }인 쿠키를 찾으세요.
login-cookie-named-step-4 = 해당 값을 두 번 클릭하여 복사하세요.
login-cookie-named-note = { $cookie } 값만 있어도 충분합니다. 네트워크 탭에서 복사한 전체 Cookie 헤더를 사용해도 됩니다.
login-guest-title = 게스트 모드
login-guest-use = 게스트 모드 사용
login-usage-consent = Sonora 사용자 수를 추정하는 데 도움을 주세요.
login-device-code = { $url }에서 이 코드를 입력하세요.
login-server-title = Subsonic 서버에 연결
login-server-detail = Subsonic 또는 OpenSubsonic 서버(Navidrome, Airsonic, Gonic 등)의 주소를 입력한 다음 서버 사용자 이름과 비밀번호로 로그인하세요. 세션은 이 기기에 유지됩니다.
login-server-hint = https://music.example.com
login-jellyfin-title = Jellyfin에 연결
login-jellyfin-detail = Jellyfin 계정으로 로그인하세요.
login-quick-connect = Quick Connect로 로그인
login-username-hint = 사용자 이름
login-password-hint = 비밀번호
login-server-submit = 연결
login-account-title = 계정 선택
login-account-detail = 이 세션에 두 개 이상의 Google 계정이 로그인되어 있습니다. Sonora에서 사용할 계정을 선택하세요.
login-profile-title = 프로필 선택
login-profile-detail = 이 계정에 두 개 이상의 프로필이 있습니다. Sonora에서 사용할 프로필을 선택하세요.
login-choose-title = { $provider }에 로그인
login-choose-detail = { $provider }에 로그인할 방법을 선택하세요.
login-providers = 음악 서비스
login-welcome-title = Sonora에 오신 것을 환영합니다!
login-welcome-detail = 나중에 설정에서 음악 서비스를 더 추가할 수 있습니다.
login-or = 또는

# album and playlist pages

# artist page
artist-eyebrow = 아티스트
artist-monthly-listeners = { $count ->
    [one] 월별 리스너 { $value }명
   *[other] 월별 리스너 { $value }명
}
artist-followers = { $count ->
    [one] 팔로워 { $value }명
   *[other] 팔로워 { $value }명
}
artist-popular = 인기 곡
artist-popular-eyebrow = 이 아티스트 둘러보기
artist-popular-empty = 이 아티스트의 재생 가능한 곡이 아직 없습니다.
artist-favorite-songs = 즐겨찾기한 곡
artist-favorite-songs-count = { $count ->
    [one] 즐겨찾기에 있는 { $count }곡
   *[other] 즐겨찾기에 있는 { $count }곡
}
artist-releases = 발매작
artist-filter-all = 전체
artist-filter-singles = 싱글
artist-filter-eps = EP
artist-appears-on = 참여 앨범
album-also-like = 추천 앨범
album-tab-artists = 아티스트
album-label = ℗ { $label }

# user profile page
user-eyebrow = 프로필
user-followers = { $count ->
    [one] 팔로워 { $value }명
   *[other] 팔로워 { $value }명
}
user-following = { $count ->
   *[other] 팔로잉 { $value }명
}
user-playlists = 공개 플레이리스트
user-playlists-empty = 공개 플레이리스트가 아직 없습니다.

# home page
home-quick-picks = 빠른 선곡
home-quick-picks-empty = 몇 곡에 좋아요를 누르면 여기에 표시됩니다.
home-recently-added = 최근 추가됨
home-recently-played = 최근 재생한 항목
home-most-played = 가장 많이 재생한 항목
home-favorite-albums = 즐겨찾기한 앨범
home-artists = 아티스트
home-collection-albums = 내 컬렉션의 앨범

# powerbar (quick search)
powerbar-title = 빠른 검색
powerbar-open = 열기
powerbar-song = 곡으로 이동

# search page
search-placeholder = 어떤 음악을 듣고 싶으신가요?
search-browse = 모두 둘러보기
search-submit = 검색
search-open-link = Sonora에서 링크 열기
search-recent = 최근 검색어
search-forget = 기록에서 제거
genre-empty = 여기에 표시할 항목이 아직 없습니다.
search-best-match = 상위 결과
search-no-matches = 일치하는 항목 없음
search-results = 결과
search-artists = 아티스트
search-albums-playlists = 앨범 및 플레이리스트
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] 라이브러리에 { $count }곡 있음
       *[other] 라이브러리에 { $count }곡 있음
    }

# song page
song-eyebrow = 곡
song-view-album = 앨범 보기
song-about = 이 곡에 대하여
song-album = 앨범
song-released = 발매일
song-streams = 스트리밍 수
song-position = 트랙 위치
song-label = 레이블
song-popularity = 인기도
song-disc-track = 디스크 { $disc }, 트랙 { $track }
song-track = 트랙 { $track }
song-credits = 크레딧
song-performed-by = 아티스트
song-details = 장르 및 상세 정보
song-genres = 장르
song-language = 언어
song-content = 콘텐츠 등급
song-explicit = 성인용(Explicit)
song-local = 로컬 파일
song-clean = 일반(Clean)
artist-about = 아티스트 소개
artist-about-fallback = 아티스트의 인기 곡과 발매작을 둘러보세요.
artist-about-open = 아티스트로 이동
song-copyright = © { $notice }
track-row-name = { $artists }의 { $title }

# tags
tags-edit-title = 태그 편집
tags-sheet-song = 곡
tags-sheet-album = 앨범
tags-sheet-details = 상세 정보
tags-title = 제목
tags-artist = 아티스트
tags-artists = 아티스트 목록
tags-add-artist = 아티스트 추가
tags-remove-artist = 이 아티스트 제거
tags-track = 트랙 번호
tags-track-total = 총 트랙 수
tags-disc = 디스크 번호
tags-disc-total = 총 디스크 수
tags-album = 앨범
tags-album-artist = 앨범 아티스트
tags-year = 연도
tags-genre = 장르
tags-composer = 작곡가
tags-publisher = 레이블/배급사
tags-isrc = ISRC
tags-comment = 설명
