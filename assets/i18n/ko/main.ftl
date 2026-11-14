# common
common-on = 켬
common-off = 끔
common-left = 왼쪽
common-right = 오른쪽
common-search = 검색
common-unknown = 알 수 없음
common-not-provided = 제공되지 않음
common-not-available = 사용할 수 없음
common-cancel = 취소
common-save = 저장
common-delete = 삭제
common-play = 재생
common-more = 더보기
common-previous = 이전
common-next = 다음
common-dismiss = 닫기
common-clear = 지우기
option-variants = { $name }의 다른 버전
table-sorted-ascending = 오름차순 정렬됨
table-sorted-descending = 내림차순 정렬됨
range-from = 시작
range-to = 끝
rail-back = 뒤로 스크롤
rail-forward = 앞으로 스크롤
number-group = { "," }

# navigation
nav-history = 재생 기록
nav-home = 홈
nav-search = 검색
nav-library = 내 라이브러리
nav-settings = 설정
nav-songs = 곡
nav-albums = 앨범
nav-playlists = 플레이리스트
nav-artists = 아티스트
nav-local = 로컬 음악
nav-back = 뒤로
nav-forward = 앞으로
nav-sidebar = 사이드바 전환
nav-sidebar-right = 가사 및 재생 목록 표시/숨기기
nav-pinned = 고정됨
nav-unpin = 고정 해제
nav-pin-hint = 고정하려면 여기에 놓으십시오.
library-liked-songs = 즐겨찾기
library-no-songs = 즐겨찾기한 곡이 아직 없습니다.
library-no-albums = 저장된 앨범이 아직 없습니다.
library-no-playlists = 플레이리스트가 아직 없습니다.
library-no-artists = 즐겨찾기한 아티스트가 아직 없습니다.
library-no-local-songs = 가져온 곡을 찾을 수 없습니다.
library-no-local-albums = 가져온 앨범을 찾을 수 없습니다.
library-no-local-artists = 가져온 아티스트를 찾을 수 없습니다.
library-no-local-playlists = 로컬 플레이리스트가 아직 없습니다.
library-no-catalog-songs = 곡을 찾을 수 없습니다.
library-no-catalog-albums = 앨범을 찾을 수 없습니다.
library-no-catalog-artists = 아티스트를 찾을 수 없습니다.
library-no-matches = 일치하는 항목 없음
library-not-loaded = 라이브러리를 불러오지 못했습니다.
library-scanning = 폴더 검색 중…
library-scanning-found = 폴더 검색 중… 지금까지 { $found }개 찾음
library-scanning-progress = 음악 읽는 중… { $found }개 중 { $read }개
library-part-not-loaded = 라이브러리의 이 부분을 불러오지 못했습니다.
library-local-unconfigured = 로컬 라이브러리 구성

# app menu
app-refresh = 새로고침
downloads-active = 다운로드 중
downloads-left = { $count ->
    [one] { $count }곡 남음
   *[other] { $count }곡 남음
}
downloads-eta = 약 { $minutes }분 남음
downloads-songs = 곡
downloads-cancel = 다운로드 취소
downloads-cancel-all = 모든 다운로드 취소
app-sign-out = 로그아웃
app-quit = 종료
app-settings = 설정…
app-hide = Sonora 숨기기
app-hide-others = 기타 숨기기
app-show-all = 모두 보기
app-edit = 편집
app-cut = 잘라내기
app-copy = 복사
app-paste = 붙여넣기
app-select-all = 모두 선택
app-window = 창
app-close-window = 창 닫기
app-minimize = 최소화
app-zoom = 확대/축소
window-maximize = 최대화
window-restore = 이전 크기로 복원

# tray menu
tray-show = Sonora 표시
tray-play = 재생
tray-pause = 일시정지

# table columns
column-played-at = 재생한 시간
column-index = #
column-title = 제목
column-artist = 아티스트
column-album = 앨범
column-genre = 장르
column-date-added = 추가된 날짜
column-added-by = 추가한 사용자
column-modified = 수정일
column-length = 길이
column-plays = 재생 수
column-name = 이름
column-owner = 소유자
column-year = 연도
column-tracks = 트랙

# track menu
menu-add-to-playlist = 플레이리스트에 추가
menu-add-tracks-to-playlist = { $count ->
    [one] 플레이리스트에 { $count }트랙 추가
   *[other] 플레이리스트에 { $count }트랙 추가
}
menu-new-playlist = 새 플레이리스트
menu-import-playlist = 플레이리스트 가져오기…
menu-edit-tags = 태그 편집
menu-no-playlists = 플레이리스트 없음
menu-search-playlists = 플레이리스트 검색
menu-save-library = 라이브러리
menu-add-to-library = 즐겨찾기에 추가
menu-remove-from-library = 즐겨찾기에서 제거
menu-remove-from-playlist = 플레이리스트에서 제거
menu-remove-tracks-from-playlist = { $count ->
    [one] 플레이리스트에서 { $count }트랙 제거
   *[other] 플레이리스트에서 { $count }트랙 제거
}
menu-remove-from-history = 재생 기록에서 제거
menu-remove-tracks-from-history = { $count ->
    [one] 재생 기록에서 { $count }트랙 제거
   *[other] 재생 기록에서 { $count }트랙 제거
}
menu-delete-track-file = 트랙 파일 삭제
menu-delete-track-files = 트랙 파일 { $count }개 삭제
menu-download = 다운로드
menu-remove-download = 다운로드 항목 제거
menu-downloading = 다운로드 중입니다. 중지하려면 클릭
menu-play-next = 바로 다음에 재생
menu-play-tracks-next = { $count ->
    [one] { $count }트랙을 바로 다음에 재생
   *[other] { $count }트랙을 바로 다음에 재생
}
menu-add-to-queue = 재생 목록에 추가
menu-add-tracks-to-queue = { $count ->
    [one] 재생 목록에 { $count }트랙 추가
   *[other] 재생 목록에 { $count }트랙 추가
}
menu-play-last = 맨 마지막에 재생
menu-play-tracks-last = { $count ->
    [one] { $count }트랙을 맨 마지막에 재생
   *[other] { $count }트랙을 맨 마지막에 재생
}
menu-song-radio = 곡 라디오로 이동
menu-go-to-album = 앨범으로 이동
menu-go-to-artist = 아티스트로 이동
menu-go-to-playlist = 플레이리스트로 이동
menu-view-details = 상세 정보 보기
menu-copy-link = 링크 복사
menu-cut = 잘라내기
menu-copy = 복사
menu-paste = 붙여넣기
menu-select-all = 모두 선택
menu-remove-from-queue = 재생 목록에서 제거
menu-edit-playlist = 플레이리스트 편집
menu-delete-playlist = 플레이리스트 삭제
menu-export-playlist = 플레이리스트 내보내기…
menu-add-playlist-to-library = 라이브러리에 추가
menu-remove-playlist-from-library = 라이브러리에서 제거
menu-make-playlist-public = 공개로 전환
menu-make-playlist-private = 비공개로 전환
menu-play-artist = 아티스트 재생

# playlist editor
playlist-name-placeholder = 플레이리스트 이름
playlist-create-title = 플레이리스트 만들기
playlist-edit-title = 플레이리스트 편집
playlist-cover-choose = 사진 선택
playlist-cover-remove = 커버 제거
playlist-cover-unopenable = Sonora에서 이 파일을 열 수 없습니다.
playlist-cover-too-large = 이 사진은 40 MB보다 큽니다.
playlist-cover-not-a-picture = JPEG, PNG 또는 WebP 이미지 파일이 아닙니다.
playlist-delete-title = 플레이리스트 삭제
playlist-delete-confirm = “{ $name }”을(를) 삭제하시겠습니까? 이 작업은 되돌릴 수 없습니다.
playlist-again-title = 다시 추가하시겠습니까?
playlist-again-confirm = 이 트랙은 이미 “{ $name }”에 있습니다. 중복해서 추가하시겠습니까?
playlist-again-add = 다시 추가

# confirm
confirm-remove-library-title = 라이브러리에서 제거
confirm-remove-playlist-title = 플레이리스트에서 제거
confirm-remove-history-title = 재생 기록에서 제거
confirm-remove-songs = { $count ->
    [one] 라이브러리에서 이 곡을 제거하시겠습니까?
   *[other] 라이브러리에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-playlist-songs = { $count ->
    [one] 플레이리스트에서 이 곡을 제거하시겠습니까?
   *[other] 플레이리스트에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-history-songs = { $count ->
    [one] 재생 기록에서 이 곡을 제거하시겠습니까?
   *[other] 재생 기록에서 { $count }곡을 제거하시겠습니까?
}
confirm-remove-albums = { $count ->
    [one] 라이브러리에서 이 앨범을 제거하시겠습니까?
   *[other] 라이브러리에서 앨범 { $count }개를 제거하시겠습니까?
}
confirm-remove-artists = { $count ->
    [one] 즐겨찾기에서 이 아티스트를 제거하시겠습니까?
   *[other] 즐겨찾기에서 아티스트 { $count }명을 제거하시겠습니까?
}
confirm-remove-playlists = { $count ->
    [one] 라이브러리에서 이 플레이리스트를 제거하시겠습니까?
   *[other] 라이브러리에서 플레이리스트 { $count }개를 제거하시겠습니까?
}
confirm-delete-track-files-title = 트랙 파일을 삭제하시겠습니까?
confirm-delete-track-files = 파일이 디스크에서 영구적으로 삭제됩니다. 이 작업은 되돌릴 수 없습니다.

# queue panel
queue-title = 재생 목록
queue-history = 재생 기록
queue-now-playing = 지금 재생 중
queue-from = 재생 출처:
queue-from-radio = { $name } 라디오
queue-next-in-queue = 다음 대기열
queue-up-next = 다음 재생할 곡
queue-reset = 재설정
queue-clear = 비우기
queue-added-by = { $name }님이 추가함
queue-empty = 재생 목록이 비어 있습니다.
queue-similar = 비슷한 트랙
queue-radio = 비슷한 트랙 자동 재생
queue-return-playing = 지금 재생 중인 화면으로 돌아가기

# player bar
player-nothing-playing = 재생 중인 곡 없음
player-percent = { $value }%
player-shuffle = 셔플
player-repeat = 반복
player-repeat-all = 전체 반복
player-repeat-one = 한 곡 반복
player-mute = 음소거
player-unmute = 음소거 해제
player-previous = 이전 트랙
player-next = 다음 트랙
player-fullscreen = 지금 재생 중
player-fullscreen-leave = 지금 재생 중 화면 나가기
player-time-left = -{ $time }
player-time-left-spoken = { $time } 남음
devices-this-computer = 이 컴퓨터
devices-jam-join = Jam 참여…
devices-jam-join-title = Jam 참여
devices-jam-join-detail = 친구가 공유한 링크를 붙여넣으세요.
devices-jam-paste = 붙여넣기
devices-jam-link-hint = Jam 링크
devices-jam-start = Jam 시작
devices-jam-copy = Jam 링크 복사
devices-jam-code-menu = QR 코드 표시
devices-jam-code-title = 스캔하여 { $host }님의 Jam에 참여
devices-jam-code-title-anonymous = 스캔하여 Jam에 참여
devices-jam-code-close = 닫기
devices-jam-leave = Jam 나가기
devices-jam-end = Jam 종료
devices-jam-more = +{ $count }
devices-jam-host = 호스트
devices-jam-you = 나
devices-jam-remove = 내보내기
devices-jam-people = { $count ->
    [one] 이 Jam에 { $count }명 참여 중
   *[other] 이 Jam에 { $count }명 참여 중
    }
devices-jam-remove-member = { $name } 내보내기
devices-jam-invite-title = { $host }님의 Jam에 참여하시겠습니까?
devices-jam-invite-title-anonymous = 이 Jam에 참여하시겠습니까?
devices-jam-invite-detail = { $count ->
    [one] 이미 { $count }명이 이 Jam에 참여하고 있습니다.
   *[other] 이미 { $count }명이 이 Jam에 참여하고 있습니다.
}
devices-jam-invite-join = 참여
player-sessions = 세션
devices-spotify-jams = Spotify Jam
members-title = 멤버
members-empty = 함께 듣는 사람을 보려면 세션을 시작하거나 참여하세요.
devices-session-start = 세션 시작
devices-session-join = 세션 참여…
devices-session-join-title = 세션 참여
devices-session-join-detail = 친구가 공유한 코드를 입력하세요.
devices-session-join-confirm = 참여
devices-session-code = 세션 코드
devices-session-code-menu = 세션 코드 표시
devices-session-code-title = 내 세션 코드
devices-session-code-detail = { $provider }에 로그인한 누구나 이 코드로 참여할 수 있습니다.
devices-session-code-close = 완료
devices-session-copy = 코드 복사
devices-session-leave = 세션 나가기
devices-session-end = 세션 종료
devices-session-reconnecting = 다시 연결하는 중…
devices-session-people = { $count ->
    [one] 이 세션에 { $count }명 참여 중
   *[other] 이 세션에 { $count }명 참여 중
    }
fullscreen-artwork = 아트워크
fullscreen-panels = 지금 재생 중 보기

# filters
filter-history = 재생 기록 필터링
history-empty = 재생한 트랙이 여기에 표시됩니다.
history-not-loaded = 재생 기록을 불러올 수 없습니다.
history-clear = 기록 지우기
history-clear-title = 재생 기록 지우기
history-clear-confirm = 이 기기에서 모든 재생 기록이 삭제됩니다. 이 작업은 되돌릴 수 없습니다.
filter-library = 라이브러리 필터링
filter-album = 앨범 트랙 필터링
filter-reset = 필터 초기화
filter-duration = 재생 시간
filter-year = 연도
filter-hide-explicit = 성인용 콘텐츠 숨기기
filter-playable = 재생 가능한 곡만
filter-favorites = 즐겨찾기만
filter-downloaded = 다운로드됨
filter-owned = 내가 만든 항목

# view
view-list = 목록
view-cards = 그리드

# toolbar
tool-sort = 정렬
tool-filters = 필터

# login
login-signed-out = 음악 라이브러리를 불러오려면 로그인하세요.
login-restoring = 저장된 세션 확인 중…
login-authorizing = 브라우저에서 인증을 기다리는 중…
login-signed-in = { $name }(으)로 로그인됨
login-failed-title = 로그인 실패
login-problem-region = 현재 계신 국가에서는 Spotify 세션을 열 수 없습니다. 거주 국가에서 로그인하거나 Spotify 계정의 국가 설정을 변경하세요.
login-problem-credentials = 이 서비스에 저장된 세션이 더 이상 유효하지 않습니다. 계속하려면 다시 로그인하세요.
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
login-window-title = { $provider }에 로그인
login-use = { $provider } 사용
login-guest-title = 게스트 모드
login-guest-use = 게스트 모드 사용
login-guest-detail = 계정 없이 둘러보고 재생할 수 있습니다. 라이브러리, 좋아요 및 플레이리스트는 이용할 수 없습니다.
login-usage-consent = Sonora 사용자 수를 추정하는 데 도움을 주세요.
login-device-code = { $url }에서 이 코드를 입력하세요.
login-server-title = Subsonic 서버에 연결
login-server-detail = Subsonic 또는 OpenSubsonic 서버(Navidrome, Airsonic, Gonic 등)의 주소를 입력한 다음 서버 사용자 이름과 비밀번호로 로그인하세요. 세션은 이 기기에 유지됩니다.
login-server-label = 서버 주소
login-server-hint = https://music.example.com
login-username-hint = 사용자 이름
login-password-hint = 비밀번호
login-server-submit = 연결
login-account-title = 계정 선택
login-account-detail = 이 세션에 두 개 이상의 Google 계정이 로그인되어 있습니다. Sonora에서 사용할 계정을 선택하세요.
login-choose-title = { $provider }에 로그인
login-choose-detail = { $provider }에 로그인할 방법을 선택하세요.
login-providers = 음악 서비스

# album and playlist pages
detail-playlist = 플레이리스트

# play button
play-pause = 일시정지
play-resume = 다시 재생
play-shuffle = 셔플

# artist page
artist-eyebrow = 아티스트
artist-monthly-listeners = { $count ->
    [one] 월별 리스너 { $value }명
   *[other] 월별 리스너 { $value }명
}
artist-popular = 인기 곡
artist-popular-eyebrow = 이 아티스트 둘러보기
artist-popular-empty = 이 아티스트의 재생 가능한 곡이 아직 없습니다.
artist-popular-more = 모두 보기
artist-popular-less = 접기
artist-favorite-songs = 즐겨찾기한 곡
artist-favorite-songs-count = { $count ->
    [one] 즐겨찾기에 있는 { $count }곡
   *[other] 즐겨찾기에 있는 { $count }곡
}
artist-releases = 발매작
artist-releases-more = 모두 보기
artist-releases-less = 접기
artist-filter-all = 전체
artist-filter-albums = 앨범
artist-filter-singles = 싱글
artist-filter-eps = EP
artist-appears-on = 참여 앨범
album-also-like = 추천 앨범
album-tab-albums = 앨범
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

# release kinds
release-album = 앨범
release-single = 싱글
release-compilation = 컴필레이션
release-ep = EP
release-audiobook = 오디오북
release-podcast = 팟캐스트
release-meta = { $year } • { $kind }

# home page
home-quick-picks = 빠른 선곡
home-quick-picks-empty = 몇 곡에 좋아요를 누르면 여기에 표시됩니다.
home-recently-added = 최근 추가됨
home-recently-played = 최근 재생한 항목
home-playlists = 플레이리스트
home-favorite-albums = 즐겨찾기한 앨범
home-artists = 아티스트
home-collection-albums = 내 컬렉션의 앨범

# powerbar (quick search)
powerbar-title = 빠른 검색
powerbar-placeholder = 어떤 음악을 듣고 싶으신가요?
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
search-songs = 곡
search-artists = 아티스트
search-albums-playlists = 앨범 및 플레이리스트
search-tag = { $kind } ·
search-saved =
    { $count ->
        [one] 라이브러리에 { $count }곡 있음
       *[other] 라이브러리에 { $count }곡 있음
    }
kind-song = 곡
kind-artist = 아티스트
kind-album = 앨범
kind-playlist = 플레이리스트

# song page
song-eyebrow = 곡
song-view-album = 앨범 보기
song-loading = 곡 정보 불러오는 중…
song-about = 이 곡에 대하여
song-album = 앨범
song-released = 발매일
song-streams = 스트리밍 수
song-position = 트랙 위치
song-label = 레이블
song-popularity = 인기도
song-popularity-value = { $value }%
song-disc-track = 디스크 { $disc }, 트랙 { $track }
song-track = 트랙 { $track }
song-credits = 크레딧
song-performed-by = 아티스트
song-details = 장르 및 상세 정보
song-genres = 장르
song-language = 언어
song-content = 콘텐츠 등급
song-explicit = 성인용(Explicit)
song-clean = 일반(Clean)
artist-about = 아티스트 소개
artist-about-fallback = 아티스트의 인기 곡과 발매작을 둘러보세요.
artist-about-open = 아티스트로 이동
song-copyright = © { $notice }
track-row-name = { $artists }의 { $title }

# song languages
language-ar = 아랍어
language-de = 독일어
language-en = 영어
language-es = 스페인어
language-fr = 프랑스어
language-hi = 힌디어
language-it = 이탈리아어
language-ja = 일본어
language-ko = 한국어
language-pt = 포르투갈어
language-ru = 러시아어
language-tr = 튀르키예어
language-uk = 우크라이나어
language-zh = 중국어
language-zxx = 언어 콘텐츠 없음

# counts
count-songs =
    { $count ->
        [one] { $count }곡
       *[other] { $count }곡
    }
count-tracks =
    { $count ->
        [one] { $count }트랙
       *[other] { $count }트랙
    }

# running times
runtime-seconds = { $seconds }초
runtime-minutes = { $minutes }분 { $seconds }초
runtime-hours = { $hours }시간 { $minutes }분

# dates
date-just-now = 방금 전
date-minute-ago = 1분 전
date-minutes-ago = { $count }분 전
date-today = 오늘 { $time }
date-yesterday = 어제 { $time }
date-time = { $date }, { $time }
date-full = { $year }년 { $month } { $day }일
month-1 = 1월
month-2 = 2월
month-3 = 3월
month-4 = 4월
month-5 = 5월
month-6 = 6월
month-7 = 7월
month-8 = 8월
month-9 = 9월
month-10 = 10월
month-11 = 11월
month-12 = 12월

# settings
settings-search = 설정 검색
settings-search-results = 검색 결과
# where a search result lives: a drill-in page under the page it opens from
settings-result-place = { $parent } › { $page }
settings-page-providers = 서비스 제공업체
settings-page-appearance = 테마
settings-page-player-bar = 플레이어 바
settings-player-bar-layout = 레이아웃
settings-player-bar-layout-detail = 트랙 정보, 재생 컨트롤 및 진행률 표시줄 배치 방식
settings-player-bar-classic = 클래식
settings-player-bar-compact = 컴팩트
settings-player-bar-centered = 가운데 정렬
settings-player-bar-surface = 표면 스타일
settings-player-bar-transparent = 투명 플레이어 바
settings-player-bar-transparent-detail = 반투명 플레이어 바 아래로 페이지와 사이드바가 스크롤되도록 합니다.
settings-player-bar-transparent-needs-blur = 반투명 플레이어 바 아래로 페이지가 스크롤되게 하려면 UI 블러 효과를 켜세요.
settings-player-bar-time = 시간 표시
settings-player-bar-time-left = 남은 시간 표시
settings-player-bar-time-left-detail = 전체 길이 대신 곡의 남은 시간을 카운트다운으로 표시합니다.
settings-page-now-playing = 지금 재생 중
settings-page-lyrics = 가사
settings-page-playback = 재생
settings-page-sound = 사운드
settings-page-storage = 저장 공간
settings-page-discord = Discord
settings-page-scrobbling = 스크로블링
settings-page-general = 일반
settings-page-about = 정보
settings-tab-integrations = 연동
settings-about-name = Sonora
settings-page-lyrics-sources = 가사 소스
settings-page-music-folders = 음악 폴더
settings-page-keybinds = 단축키
keybinds-group-playback = 재생
keybinds-group-navigation = 탐색
keybinds-group-view = 보기
keybinds-change = { $action } 단축키 변경
keybinds-recording = 키를 누르세요…
keybinds-none = 설정되지 않음
keybinds-reset = 기본값으로 재설정
keybinds-reset-all = 모든 단축키 재설정
keybinds-reset-all-button = 재설정
keybind-toggle-playback = 재생 또는 일시정지
keybind-song-previous = 이전 트랙
keybind-song-next = 다음 트랙
keybind-toggle-shuffle = 셔플
keybind-toggle-repeat = 반복
keybind-navigate-back = 뒤로 가기
keybind-navigate-forward = 앞으로 가기
keybind-open-search = 검색
keybind-toggle-powerbar = 빠른 검색
keybind-open-filter = 페이지 필터링
keybind-open-settings = 설정 열기
keybind-refresh = 새로고침
keybind-toggle-fullscreen = 지금 재생 중
keybind-toggle-queue = 재생 목록 표시
keybind-toggle-lyrics = 가사 표시
keybind-toggle-window-fullscreen = 전체 화면 창
keybind-increase-font-size = 글자 크게
keybind-decrease-font-size = 글자 작게
keybind-reset-font-size = 기본 글자 크기
settings-group-services = 스트리밍 서비스
settings-group-local-files = 로컬 파일
settings-group-background = 배경
settings-group-controls = 컨트롤
settings-group-display = 디스플레이
settings-group-sources = 소스
settings-group-lyrics-order = 우선순위
settings-group-playing = 재생 중
settings-group-cache = 재생된 트랙
settings-group-downloads = 다운로드
settings-group-protected = 보호된 재생
settings-group-soloist = Spotify 플레이어
settings-group-output = 출력
settings-group-status = 상태
settings-group-startup = 시작
settings-group-context-menus = 컨텍스트 메뉴
key-ctrl = Ctrl
key-super = Super
key-alt = Alt
key-shift = Shift
key-escape = Esc
key-enter = Enter
key-space = Space
key-tab = Tab
key-backspace = Backspace
key-delete = Delete
settings-lyrics-sources = 가사 소스
settings-local-folder-count = { $count ->
    [one] 폴더 { $count }개
   *[other] 폴더 { $count }개
    }
settings-local-folder-none = 폴더가 아직 없습니다.
settings-license-short = GPL-3.0 이상
settings-theme = 테마
settings-theme-detail = 애플리케이션 색상 팔레트 선택
settings-opacity = 불투명도
settings-opacity-detail = 앱 배경의 불투명도 조절
settings-opacity-value = { $percent }%
settings-theme-folder = 폴더 열기
settings-theme-explore = 커뮤니티 테마 둘러보기
settings-theme-explore-detail = 다른 사람들이 만든 테마를 찾아 한 번의 클릭으로 설치하세요.
settings-theme-delete = 테마 삭제
settings-theme-delete-title = { $name } 테마를 삭제하시겠습니까?
settings-theme-delete-detail = 테마 폴더에서 제거됩니다. 이 작업은 되돌릴 수 없습니다.
settings-theme-unavailable = 이 테마는 적응형 테마가 비활성화된 경우에만 적용할 수 있습니다.
settings-adaptive = 적응형 테마
settings-adaptive-detail = 재생 중인 앨범의 아트워크 색상으로 팔레트를 물들입니다.
settings-ambient = 앰비언트 배경
settings-ambient-detail = 커버에서 추출한 색상으로 '지금 재생 중' 화면을 채웁니다.
settings-ambient-motion = 앰비언트 모션
settings-ambient-motion-detail = 앰비언트 배경 색상을 고정하지 않고 부드럽게 흐르도록 합니다.
settings-visualizer = 비주얼라이저
settings-visualizer-detail = '지금 재생 중' 화면의 아트워크 뒤에 스펙트럼이 그려지는 방식
settings-visualizer-style-none = 끔
settings-visualizer-style-bars = 막대형
settings-visualizer-style-wave = 파형
settings-visualizer-style-both = 막대형 및 파형
settings-visualizer-screens = 표시 위치
settings-visualizer-screens-detail = 스펙트럼이 그려질 '지금 재생 중' 보기 화면
settings-visualizer-screens-pick = 보기 선택
settings-visualizer-absolute = 볼륨 무시
settings-visualizer-absolute-detail = Sonora의 재생 음량과 관계없이 트랙 자체의 음량 레벨로 스펙트럼을 그립니다.
settings-preview-title = 곡 제목
settings-preview-artist = 아티스트
settings-preview-album = 앨범
settings-fullscreen-controls-autohide = 컨트롤 숨기기
settings-fullscreen-controls-autohide-detail = 마우스 포인터가 움직이지 않으면 '지금 재생 중' 화면의 재생 컨트롤을 서서히 숨깁니다.
settings-os-fullscreen = OS 전체 화면 사용
settings-os-fullscreen-detail = F11 키를 누른 것처럼 '지금 재생 중' 화면이 전체 화면을 채우도록 합니다.
settings-icons = 아이콘 팩
settings-icons-detail = 인터페이스에 사용할 아이콘 세트 선택
settings-motion = 동작 줄이기
settings-motion-detail = 인터페이스 애니메이션 및 화면 전환 효과 건너뛰기
settings-pace = 애니메이션 속도
settings-pace-detail = 인터페이스 애니메이션 재생 속도
settings-saver = 비활성 창 프레임 제한
settings-saver-detail = 변경 사항은 Sonora를 다시 시작한 후 적용됩니다.
settings-corners = 모서리 둥글기
settings-corners-detail = 표면과 컨트롤의 모서리 둥글기 정도
settings-blur = UI 블러
settings-blur-detail = 메뉴와 플로팅 컨트롤 아래의 배경을 반투명하게 흐리게 처리합니다.
settings-blur-window = 창 블러
settings-blur-window-detail = 흐리게 처리된 바탕 화면 위에 창을 그립니다. 불투명도를 100% 미만으로 설정해야 합니다.
settings-font = 글꼴 크기
settings-font-detail = 기본 텍스트 크기이며, 다른 모든 요소도 이에 비례해 조정됩니다.
settings-font-value = { $size } px
settings-font-smaller = 글자 작게
settings-font-larger = 글자 크게
settings-startup = 시작 화면
settings-startup-detail = Sonora 실행 시 처음 열리는 화면
settings-startup-no-guest = 게스트 모드에서는 사용할 수 없음
settings-entries = 사이드바 항목
settings-entries-detail = 사이드바에 표시되는 섹션 목록
settings-entries-pick = 항목 선택
settings-language = 언어
settings-language-detail = Sonora 인터페이스 전반에 사용되는 언어
settings-language-system = 시스템 기본값
settings-language-search = 언어 검색
settings-language-none = 언어를 찾을 수 없습니다.
settings-typeface = 글꼴
settings-typeface-detail = Sonora 인터페이스 전반에 사용되는 서체
settings-typeface-system = 기본값
settings-typeface-search = 글꼴 검색
settings-typeface-none = 글꼴을 찾을 수 없습니다.
settings-server-side-decorations = 서버 측 창 장식
settings-server-side-decorations-detail = 컴포지터가 제목 표시줄, 테두리 및 그림자를 그리도록 합니다.
settings-typeface-loading = 불러오는 중…
settings-window-controls = 창 컨트롤
settings-window-controls-detail = 제목 표시줄에 최소화, 최대화 및 닫기 버튼 표시
settings-traffic-light-controls = 신호등 스타일 창 컨트롤
settings-traffic-light-controls-detail = 최소화, 최대화 및 닫기 버튼을 색상 원형 버튼으로 표시
settings-window-rounding = 창 모서리 둥글기
settings-window-rounding-detail = 애플리케이션 창 자체의 모서리 둥글기 정도
settings-controls-side = 컨트롤 위치
settings-controls-side-detail = 제목 표시줄에서 창 컨트롤이 위치할 방향
settings-close-to-tray = 백그라운드에서 계속 실행
settings-close-to-tray-detail = 창을 닫은 후에도 Sonora가 계속 실행되고 음악을 재생하도록 유지합니다.
settings-tray-icon = 시스템 트레이에 표시
settings-tray-icon-detail = 시스템 트레이에 재생 컨트롤이 포함된 아이콘을 표시합니다.
settings-spotify-connect = Spotify 기기로 표시
settings-spotify-connect-name = 기기 이름
settings-spotify-connect-name-detail = Spotify 앱의 기기 목록에 Sonora가 표시되는 이름
settings-spotify-connect-name-sonora = Sonora
settings-spotify-connect-name-both = Sonora (컴퓨터 이름)
settings-spotify-connect-name-computer = 컴퓨터 이름
settings-spotify-connect-name-custom = 사용자 지정
settings-spotify-connect-custom = 사용자 지정 이름
settings-spotify-connect-custom-detail = 표시할 이름입니다. 비워 두면 Sonora와 컴퓨터 이름이 함께 사용됩니다.
settings-spotify-connect-custom-hint = 기기 이름
settings-spotify-connect-detail = Spotify 앱의 기기 선택기에 Sonora를 표시하여 휴대폰에서 제어하고 Spotify에 재생 중인 항목이 표시되도록 합니다.
settings-discord = Discord에 표시
settings-discord-detail = Discord 프로필에 현재 재생 중인 트랙을 표시합니다.
settings-discord-name = 상태 이름
settings-discord-name-detail = 친구들에게 보이는 "듣는 중" 뒤에 표시될 상태 이름
settings-discord-name-sonora = Sonora
settings-discord-name-provider = 서비스 제공업체
settings-discord-name-music = 음악
settings-discord-name-title = 제목
settings-discord-name-artist = 아티스트
settings-discord-name-album-artist = 앨범 아티스트
settings-discord-name-artist-title = 아티스트 - 제목
settings-discord-show-paused = 일시정지 시에도 표시
settings-discord-show-paused-detail = 트랙이 일시정지된 동안에도 Discord 프로필에 상태를 유지합니다.
settings-discord-badge = 서비스 배지 표시
settings-discord-badge-detail = 트랙이 재생 중인 서비스의 작은 아이콘 배지를 상태에 표시합니다.
settings-discord-anonymous = 세부 정보 숨기기
settings-discord-anonymous-detail = 제목, 아티스트 또는 아트워크 없이 음악을 재생 중이라는 사실만 표시합니다.
settings-discord-hide-on-connect = Spotify Connect 사용 중 숨기기
settings-discord-hide-on-connect-detail = Sonora가 Spotify 기기로 재생되거나 다른 기기에서 재생 중일 때는 Discord 상태를 Spotify 자체 표시에 맡깁니다.
settings-discord-buttons = 버튼
settings-discord-buttons-detail = 친구들이 열 수 있도록 상태 아래에 표시할 링크입니다(하나는 해당 서비스의 트랙 링크, 하나는 Sonora 링크).
settings-discord-buttons-pick = 버튼 선택
# what screen readers call the preview of the Discord status
settings-discord-preview = Discord 상태 미리보기
settings-discord-preview-listening = { $name } 듣는 중
settings-discord-preview-music = 음악
settings-discord-preview-elapsed = { $time } 경과
settings-discord-preview-listen-on = { $provider }에서 듣기
# the Discord status when the track is left out of it
discord-listening = 음악 듣는 중
get-sonora = Sonora 다운로드
settings-fullscreen-controls-autohide-automatic = 자동
settings-fullscreen-controls-autohide-always-hidden = 항상 숨김
settings-fullscreen-controls-autohide-always-shown = 항상 표시
settings-normalisation = 음량 평준화
settings-normalisation-detail = 모든 트랙의 볼륨을 일정하게 유지합니다.
settings-gapless = 끊김 없는 재생
settings-gapless-detail = 앨범의 원래 트랙 순서대로 곡 사이에 공백 없이 이어서 재생합니다.
settings-keep-tracks = 재생한 트랙 보관
settings-keep-tracks-detail = 재생한 곡을 이 컴퓨터에만 저장하여 다시 재생할 때 새로 다운로드하지 않습니다. 이 옵션을 끄면 보관된 파일은 삭제되지만 다운로드한 곡은 그대로 유지됩니다. Spotify는 다음에 Sonora를 시작할 때 변경 사항이 적용됩니다.
settings-cache-size = 캐시 크기
settings-cache-size-detail = 재생한 트랙이 차지할 수 있는 최대 용량입니다. 가장 오래전에 재생한 곡부터 먼저 삭제되며, 다운로드 항목은 이 용량에 포함되지 않습니다.
settings-cache-size-value = { $count } GB
settings-cache-used = 사용 중인 공간
settings-cache-used-detail = 재생 후 이 컴퓨터에 보관된 트랙이 차지하는 공간입니다. 비워도 다운로드한 항목은 유지됩니다.
settings-cache-clear = 비우기
settings-downloads-used = 사용 중인 공간
settings-downloads-used-detail = 다운로드한 곡이 이 컴퓨터에서 차지하는 공간입니다.
settings-offline-mode = 오프라인 모드
settings-offline-mode-detail = 다운로드한 항목만 표시하고 재생합니다.
settings-downloads-open = 다운로드 폴더 열기
settings-downloads-open-detail = 다른 플레이어에서도 열 수 있는 오디오 파일 형태로 다운로드가 저장된 폴더를 엽니다.
settings-downloads-remove = 모두 제거
settings-size-gb = { $size } GB
settings-size-mb = { $size } MB
settings-size-measuring = 측정 중…
settings-stream-quality = 스트리밍 음질
settings-stream-quality-detail = 음질이 높을수록 소리가 더 좋지만 데이터를 더 많이 사용합니다. 구독 요금제나 트랙이 해당 음질을 지원하지 않으면 한 단계 낮은 음질로 재생됩니다.
settings-stream-quality-hi-res = 고해상도(Hi-res)
settings-stream-quality-lossless = 무손실(Lossless)
settings-stream-quality-high = 높음
settings-stream-quality-normal = 보통
settings-stream-quality-low = 낮음
settings-stream-quality-flac = FLAC, { $depth }-bit { $rate } kHz
settings-stream-quality-flac-up-to = FLAC, 최대 { $depth }-bit { $rate } kHz
settings-stream-quality-lossy = { $codec }, { $kbps } kbps
settings-stream-quality-lossy-up-to = { $codec }, 최대 { $kbps } kbps
settings-stream-format = 스트리밍 포맷
settings-stream-format-detail = 서버가 트랙을 전송하기 전에 변환할 오디오 포맷
settings-stream-format-original = 원본
settings-stream-format-mp3 = MP3
settings-stream-format-opus = Opus
settings-stream-format-aac = AAC
settings-stream-bitrate = 최대 비트레이트
settings-stream-bitrate-detail = 이보다 높은 비트레이트의 곡은 서버에서 이 값으로 낮춰 변환합니다.
settings-stream-bitrate-unlimited = 제한 없음
settings-stream-bitrate-value = { $kbps } kbps
settings-stream-bitrate-hourly = 시간당 약 { $megabytes } MB
settings-stream-bitrate-unlimited-detail = 각 파일의 원래 비트레이트 유지
settings-output-device = 출력 장치
settings-output-device-detail = Sonora가 소리를 재생할 오디오 장치
settings-output-device-system = 시스템 기본값
settings-output-device-missing = 저장된 장치 (연결되지 않음)
settings-output-device-direct = 하드웨어 직접 출력
settings-sample-rate = 샘플 레이트
settings-sample-rate-detail = 장치가 작동하는 샘플 레이트입니다. Sonora가 모든 트랙을 이 값으로 변환합니다.
settings-sample-rate-value = { $khz } kHz
settings-sample-rate-lower = 샘플 레이트 낮추기
settings-sample-rate-higher = 샘플 레이트 높이기
settings-sample-rate-change = 변경
settings-buffer = 버퍼 크기
settings-buffer-detail = Sonora가 한 번에 오디오 장치로 전달하는 데이터 양
settings-buffer-smaller = 버퍼 작게
settings-buffer-larger = 버퍼 크게
settings-match-rate = 샘플 레이트 일치
settings-match-rate-detail = 각 트랙을 고유의 원래 샘플 레이트로 재생합니다.
settings-match-rate-direct-only = '하드웨어 직접 출력' 아래의 장치가 필요합니다.
settings-output-idle = 재생 중 아님
settings-output-device-loading = 장치 불러오는 중…
settings-output-failed = 지원되지 않는 장치
settings-output-failed-detail = 오류: { $error }
settings-signal-source = 소스
settings-signal-soloist = Soloist
settings-signal-normalized = 음량 평준화
settings-signal-gain = { $db } dB
settings-signal-resampled = 리샘플링
settings-signal-khz = { $khz } kHz
settings-signal-kbps = { $kbps } kbps
settings-signal-bits = { $depth }-bit
settings-signal-source-detail = { $rate } · { $quality }
settings-signal-path = 신호 경로
settings-sleep = 취침 타이머
settings-sleep-detail = 설정한 시간이 지나면 음악이 자동으로 멈춰 잠들기 전 음악을 들을 수 있습니다.
settings-stay-awake = 재생 중 화면 켜짐 유지
settings-stay-awake-detail = 재생 중 절전 모드 전환을 방지하고 '지금 재생 중' 화면이 열려 있는 동안 화면을 켠 상태로 유지합니다.
settings-sleep-configure = 구성…
settings-sleep-off = 끔
settings-sleep-end-of-track = 현재 트랙 종료 시
settings-sleep-minutes = { $count }분
settings-widevine = Widevine 모듈
settings-widevine-detail = Apple Music 트랙은 암호화되어 있으므로 Google의 Widevine 모듈이 필요합니다. Sonora는 사용자의 동의를 받아 Google에서 다운로드한 모듈이나 이 컴퓨터의 브라우저에 이미 설치된 복사본을 사용합니다.
settings-widevine-none = Apple Music 트랙은 암호화되어 있으므로 Google의 Widevine 모듈이 필요합니다. 이 컴퓨터의 브라우저에 모듈이 없으므로 Sonora가 사용자의 동의를 받아 Google에서 다운로드할 수 있습니다.
settings-widevine-looking = 찾는 중…
settings-widevine-asking = 응답 대기 중
settings-widevine-fetching = 다운로드 중…
settings-widevine-installing = 설치 중…
settings-widevine-installed = 브라우저에서 발견됨
settings-widevine-fetched = Google에서 다운로드됨
settings-widevine-missing = 설치되지 않음
settings-widevine-configured = 환경 변수로 설정됨
settings-widevine-fetch = 다운로드
settings-widevine-uninstall = 제거
confirm-uninstall-widevine-title = Widevine 모듈을 제거하시겠습니까?
confirm-uninstall-widevine = Sonora가 Google에서 다운로드한 복사본을 삭제합니다. 다시 다운로드할 때까지 Apple Music 트랙을 재생할 수 없습니다.
settings-soloist = Soloist 사용
settings-soloist-detail = Soloist는 Spotify에서 직접 배포하는 플레이어입니다. 내장 플레이어에서 재생이 거부되는 계정에서도 작동하며 무손실 음질을 지원합니다. Spotify Premium과 개인 API 키가 필요합니다.
settings-soloist-on-detail = Spotify가 Soloist를 통해 재생되며 오디오가 시스템 출력으로 직접 전송됩니다. 이퀄라이저, 비주얼라이저 및 출력 설정은 로컬 파일에만 적용되며, Sonora는 Spotify Connect 기기 목록 관리를 Soloist에 위임합니다.
settings-soloist-looking = 확인 중…
settings-soloist-off = 설정되지 않음
settings-soloist-setting-up = 설정 중…
settings-soloist-ready = 버전 { $version }
settings-soloist-unavailable = Soloist 사용 시 이용 불가
settings-soloist-local-only = Soloist로 Spotify를 재생하는 동안 로컬 파일에만 적용됩니다.
settings-soloist-setup = 설정
settings-soloist-remove = 제거
confirm-remove-soloist-title = Soloist를 제거하시겠습니까?
confirm-remove-soloist = Sonora가 Soloist 프로그램, API 키 및 로그인 정보를 삭제합니다. Spotify는 다시 내장 플레이어를 사용하게 됩니다.
soloist-setup-title = Soloist 설정
soloist-offered = Spotify가 내장 플레이어를 통한 이 계정의 재생을 거부했습니다. Soloist를 사용하면 재생할 수 있습니다.
soloist-step-download = 다운로드
soloist-step-key = API 키
soloist-step-pair = 페어링
soloist-terms = Sonora가 Spotify에서 Soloist를 다운로드합니다. 이를 사용하면 Spotify 이용약관에 동의하는 것으로 간주됩니다.
soloist-terms-open = 약관 읽기
soloist-download = 다운로드
soloist-later = 나중에
soloist-downloading = Spotify에서 다운로드 중…
soloist-key-step-open = Spotify 개발자 대시보드의 Soloist 페이지를 엽니다.
soloist-key-step-generate = 해당 페이지에서 약관에 동의하고 API 키를 생성합니다.
soloist-key-step-paste = 아래에 생성된 키를 붙여넣으세요.
soloist-key-open = 대시보드 열기
soloist-key-hint = API 키
soloist-continue = 계속
soloist-pair-step-open = 동일한 네트워크에 연결된 휴대폰이나 컴퓨터에서 Spotify를 엽니다.
soloist-pair-step-play = 아무 곡이나 재생을 시작합니다.
soloist-pair-step-pick = 기기 목록을 열고 { $device }을(를) 선택합니다.
soloist-pair-waiting = Spotify 연결 대기 중…
soloist-pair-firewall = 기기가 표시되지 않나요? 이 컴퓨터의 방화벽에서 UDP 5353 및 TCP { $port } 포트를 허용해야 합니다.
soloist-back = 뒤로
soloist-problem-download = Soloist를 다운로드할 수 없습니다.
soloist-problem-key = Spotify에서 이 키를 승인하지 않았습니다.
soloist-problem-key-detail = 키 전체가 올바르게 복사되었는지, 해지되지 않았는지 확인한 후 다시 시도하세요.
soloist-problem-account = 다른 계정과 페어링됨
soloist-problem-account-detail = 해당 Spotify 앱이 Sonora에서 사용하는 계정이 아닌 { $name } 계정으로 로그인되어 있습니다. 본인 계정으로 로그인된 앱에서 기기를 선택하세요.
soloist-problem-waited = 기기를 선택한 사용자가 없습니다.
soloist-problem-waited-detail = 페어링은 10분 동안 대기합니다. Spotify 앱을 준비한 상태에서 다시 시도하세요.
soloist-problem-pairing = 페어링 실패
confirm-remove-downloads-title = 모든 다운로드 항목을 제거하시겠습니까?
confirm-remove-downloads = 모든 서비스에서 다운로드한 모든 곡이 이 컴퓨터에서 삭제됩니다. 다시 다운로드하기 전까지는 스트리밍으로 재생됩니다.
widevine-prompt-title = Widevine 모듈
widevine-prompt-wanted = Apple Music 트랙은 암호화되어 있으므로 Google의 Widevine 모듈이 필요합니다. 이 컴퓨터의 브라우저에 모듈이 없습니다. Sonora가 Chrome에 설치되는 것과 동일한 파일을 Google 서버에서 다운로드하여 자체 폴더에 보관할 수 있습니다.
widevine-prompt-replace = Apple Music 트랙은 암호화되어 있으므로 Google의 Widevine 모듈이 필요합니다. 이 컴퓨터의 브라우저에 모듈이 있지만, Sonora가 Chrome에 설치되는 것과 동일한 Google 공식 복사본을 다운로드하여 자체 폴더에 보관한 뒤 대신 사용할 수 있습니다.
widevine-prompt-downloading = 다운로드 중…
widevine-prompt-terms = 버전 { $version } 다운로드가 완료되었습니다. 설치하면 모듈에 대한 Google 약관에 동의하게 됩니다:
widevine-prompt-installing = 설치 중…
widevine-prompt-download = 다운로드
widevine-prompt-later = 나중에
widevine-prompt-accept = 동의 및 설치
widevine-prompt-decline = 거절
settings-equalizer = 이퀄라이저
settings-equalizer-detail = 프리셋이나 직접 만든 곡선으로 사운드를 다듬습니다.
settings-equalizer-preset-detail = 기본 제공 사운드 또는 직접 저장한 프로필
settings-equalizer-custom = 사용자 지정
settings-equalizer-flat = 플랫
settings-equalizer-bass-boost = 저음 강조
settings-equalizer-bass-reducer = 저음 감소
settings-equalizer-treble-boost = 고음 강조
settings-equalizer-vocal = 보컬
settings-equalizer-rock = 록
settings-equalizer-pop = 팝
settings-equalizer-jazz = 재즈
settings-equalizer-classical = 클래식
settings-equalizer-electronic = 일렉트로닉
settings-equalizer-acoustic = 어쿠스틱
settings-equalizer-loudness = 라우드니스
equalizer-advanced = 고급
equalizer-simple = 기본
equalizer-sound = 사운드
equalizer-profile-default = 기본값
equalizer-profile-new = 프로필 { $number }
equalizer-profile-pasted = 붙여넣은 프로필
equalizer-profile-name-hint = 프로필 이름
equalizer-save-as = 프로필로 저장
equalizer-rename = 프로필 이름 바꾸기
equalizer-delete = 프로필 삭제
equalizer-delete-confirm = “{ $name }”을(를) 삭제하시겠습니까? 이 작업은 되돌릴 수 없습니다.
equalizer-undo = 실행 취소
equalizer-redo = 다시 실행
equalizer-bass = 저음
equalizer-treble = 고음
equalizer-tone-reset = 0 dB로 초기화
equalizer-preamp = 프리앰프
equalizer-preamp-auto = 자동
equalizer-preamp-manual = 수동
equalizer-decibels = dB
equalizer-decibels-value = { $db } dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct
equalizer-no-gain = —
equalizer-filters = 필터
equalizer-no-filters = 필터가 아직 없습니다. 필터를 추가하거나 위에서 프리셋을 선택하거나 프로필을 가져오세요.
equalizer-add-filter = 필터 추가
equalizer-remove-filter = 필터 제거
equalizer-filter = 필터 { $number }
equalizer-filter-kind = 필터 { $number } 유형
equalizer-kind-peak = 피크(Peak)
equalizer-kind-low-shelf = 로우 셸프(Low shelf)
equalizer-kind-high-shelf = 하이 셸프(High shelf)
equalizer-kind-low-pass = 로우 패스(Low pass)
equalizer-kind-high-pass = 하이 패스(High pass)
equalizer-kind-notch = 노치(Notch)
equalizer-kind-band-pass = 밴드 패스(Band pass)
equalizer-kind-all-pass = 올 패스(All pass)
equalizer-transfer = 가져오기 및 내보내기
equalizer-import = 가져오기
equalizer-export = 내보내기
equalizer-import-file = 파일 가져오기…
equalizer-paste = 붙여넣기
equalizer-copy = 복사
equalizer-export-file = 파일로 내보내기…
equalizer-imported = { $name } 가져옴
equalizer-imported-skipped = { $count ->
        [one] 1줄을 제외하고 { $name } 가져옴
       *[other] { $count }줄을 제외하고 { $name } 가져옴
    }
equalizer-import-nothing = 가져올 이퀄라이저 설정을 찾지 못했습니다.
equalizer-import-too-many = 가져오기 실패: 필터 { $count ->
        [one] 1개가 켜져 있어
       *[other] { $count }개가 켜져 있어
    } 프로필에 담을 수 있는 한도를 초과했습니다.
equalizer-import-too-large = 가져오기 실패: 텍스트 크기가 1 MB를 초과하여 이퀄라이저 프로필로 너무 큽니다.
equalizer-filter-limit = 프로필에는 최대 { $max }개의 필터를 담을 수 있습니다.
equalizer-paste-empty = 클립보드에 텍스트가 없습니다.
equalizer-copied = Equalizer APO 텍스트 형식으로 프로필을 복사했습니다.
equalizer-exported = 프로필을 내보냈습니다.
equalizer-dialog-failed = 파일 대화 상자를 열 수 없습니다.
equalizer-file-unreadable = 파일을 읽을 수 없습니다.
equalizer-file-unwritable = 파일에 쓸 수 없습니다.
equalizer-value-refused = 이전 값을 유지했습니다.
equalizer-skipped-line = { $line }번째 줄: { $reason }
equalizer-skipped-more = 외 { $count ->
        [one] 1개 더 있음
       *[other] { $count }개 더 있음
    }
equalizer-skip-unknown-type = 알 수 없는 필터 유형 { $name }
equalizer-skip-unreadable = Sonora에서 읽을 수 없는 필터입니다.
equalizer-skip-no-frequency = 주파수 없음
equalizer-skip-no-gain = 게인 없음
equalizer-skip-no-width = Q 값 또는 대역폭 없음
equalizer-skip-expression = 수식으로 작성된 값입니다.
equalizer-skip-no-preamp = 프리앰프 값이 숫자가 아닙니다.
equalizer-skip-unsupported = { $name }은(는) 지원되지 않습니다.
equalizer-rules = 기기 규칙
equalizer-rule-other = 기타 모든 기기
equalizer-rule-playing = 지금 재생 중
equalizer-rule-disconnected = 연결되지 않음
equalizer-rule-replaces = 기존 규칙 대체
equalizer-rule-choose-device = 기기 선택
equalizer-mode = 모드
equalizer-preamp-level = 프리앰프 레벨
equalizer-no-gain-spoken = 게인 없음
equalizer-filter-frequency = 필터 { $number } 주파수
equalizer-filter-gain = 필터 { $number } 게인
equalizer-filter-width = 필터 { $number } 대역폭
equalizer-graph = 주파수 응답
equalizer-graph-detail = { $count ->
    [0] 필터 없음
    [one] 필터 1개
   *[other] 필터 { $count }개
    }, 프리앰프 { $db } dB
equalizer-rule-device = 기기
equalizer-rule-choose-sound = 사운드 프로필 선택
equalizer-rule-add = 규칙 추가
equalizer-rule-remove = 규칙 제거
equalizer-output-port = { $device } · { $port }
signal-path-codec-alac = ALAC
signal-path-codec-pcm = PCM
signal-path-codec-other = 기타 코덱
equalizer-invalid-frequency = 주파수는 { $low }~{ $high } Hz 사이여야 합니다.
equalizer-invalid-gain = 게인은 { $low }~{ $high } dB 사이여야 합니다.
equalizer-invalid-q = Q 값은 { $low }~{ $high } 사이여야 합니다.
equalizer-invalid-octaves = 대역폭은 { $low }~{ $high } 옥타브 사이여야 합니다.
equalizer-invalid-shelf-octaves = 셸프 필터는 대역폭이 아닌 Q 값을 사용합니다.
equalizer-invalid-preamp = 프리앰프는 { $low }~{ $high } dB 사이여야 합니다.
equalizer-overload = { $db } dB 클리핑 발생
equalizer-overload-slight = 0.1 dB 미만 클리핑 발생
equalizer-graph-preamp = { $db } dB
equalizer-lower-preamp = 프리앰프 낮추기
equalizer-level = 레벨
equalizer-status-bit-perfect = 비트 퍼펙트
equalizer-status-bit-perfect-detail = Sonora에서 비트 퍼펙트로 출력됨
equalizer-status-server = 이후 사운드 서버에서 { $khz } kHz로 변환됩니다.
equalizer-status-altered = 비트 퍼펙트가 아닌 이유:
equalizer-status-separator = {", "}
equalizer-status-more = +{ $count }
equalizer-status-equalized = EQ
equalizer-status-volume = 볼륨
equalizer-status-normalized = 음량 평준화
equalizer-status-direct = Sonora를 거치지 않고 Soloist에서 직접 재생됨
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] 모노
    [2] 스테레오
   *[other] { $from }채널
} → { $to ->
    [1] 모노
    [2] 스테레오
   *[other] { $to }채널
}
settings-lyrics = 가사 표시
settings-lyrics-detail = 끄면 모든 곳에서 가사를 숨기고 가사 검색을 중단합니다.
settings-panel-lyrics-size = 패널 내 글자 크기
settings-panel-lyrics-size-detail = 기본 글꼴 크기에 더해지는 측면 패널 가사 텍스트의 크기
settings-fullscreen-lyrics-size = '지금 재생 중' 화면 글자 크기
settings-fullscreen-lyrics-size-detail = 기본 글꼴 크기에 더해지는 '지금 재생 중' 화면 가사 텍스트의 크기
settings-lyrics-size-value = { $size }%
settings-lyrics-for-local-files = 로컬 파일 가사 찾기
settings-lyrics-for-local-files-detail = 로컬 파일의 메타데이터를 사용하여 인터넷에서 가사를 가져옵니다.
settings-artwork-for-local-files = 로컬 파일 아트워크 찾기
settings-artwork-for-local-files-detail = 로컬 파일의 메타데이터를 사용하여 Discord 상태에 표시할 커버를 Deezer에서 찾습니다.
settings-discord-upload-covers = 커버 업로드
settings-discord-upload-covers-detail = 온라인에서 찾은 이미지 대신 정확한 앨범 커버가 상태에 표시되도록 서버나 로컬 파일의 커버를 uguu.se에 3시간 동안 업로드합니다.
settings-karaoke-lyrics = 노래방 스타일 가사
settings-karaoke-lyrics-detail = 타이밍 정보가 있는 경우 가사를 단어별로 강조 표시합니다.
settings-blur-lyrics = 비활성 줄 흐리게 처리
settings-blur-lyrics-detail = 가사 패널에서 이전 줄과 다가오는 줄을 흐리게 표시합니다.
settings-lyrics-sample-first = 임의의 데이터
settings-lyrics-sample-second = 여기에 재미있는 문구를 입력하세요.
settings-lyrics-sample-japanese = ここに面白い文章を入れてください
settings-lyrics-sample-romanized = koko ni omoshiroi bunshou o irete kudasai
settings-lyrics-sample-fourth = 이 일본어 문장이 자연스럽길 바랍니다.
settings-lyrics-sample-fifth = 다섯 번째 줄
settings-romanized-lyrics = 로마자 발음 표기
settings-romanized-lyrics-detail = 선택한 문자 체계에 대해 로컬에서 생성된 발음 표기를 표시합니다.
settings-romanization-writing-systems = 문자 체계
settings-lyrics-priority = 제공업체 우선순위
settings-lyrics-priority-detail = 동기화된 가사를 보유한 가장 높은 우선순위의 소스에서 가사를 표시합니다.
settings-lyrics-word-synced-first = 단어별 동기화 가사 우선
settings-lyrics-word-synced-first-detail = 목록에서 우선순위가 낮은 소스의 가사라도 단어 단위로 타이밍이 맞춰진 가사를 먼저 표시합니다.
settings-lyrics-providers-detail = 가사를 검색할 서비스를 선택하세요.
settings-lyrics-providers-selected = { $count ->
    [one] { $count }개 선택됨
   *[other] { $count }개 선택됨
    }
settings-lyrics-provider-local = 로컬 파일
settings-lyrics-provider-subsonic = Subsonic
settings-lyrics-provider-spotify = Spotify
settings-lyrics-provider-youtube = YouTube Music
settings-lyrics-provider-apple-music = Apple Music
settings-lyrics-provider-musixmatch = Musixmatch
settings-lyrics-provider-lrclib = LRCLIB
settings-lyrics-provider-kugou = Kugou
settings-lyrics-provider-netease = NetEase
settings-romanization-japanese = 일본어
settings-romanization-chinese = 중국어
settings-romanization-korean = 한국어
settings-romanization-cyrillic = 키릴 문자
settings-romanization-greek = 그리스 문자
settings-romanization-arabic = 아랍 문자
settings-romanization-other = 기타 문자 체계
settings-group-window = 창
settings-group-text = 텍스트
settings-group-motion = 동작
settings-group-streaming = 스트리밍
settings-group-spotify-connect = Spotify Connect
settings-group-sessions = 세션
settings-sessions-button = 세션 버튼 표시
settings-sessions-official = 공식 Sonora 세션 서버 사용
settings-sessions-server = 서버 주소
settings-sessions-server-hint = wss://sessions.example.com
settings-sessions-save = 저장
settings-adaptive-menu = 적응형 컨텍스트 메뉴
settings-adaptive-menu-detail = 앨범이나 아티스트처럼 해당 행에 이미 표시된 항목은 메뉴에서 제외합니다.
settings-accounts = 스트리밍 서비스
settings-accounts-detail = 이 기기에서 재생할 수 있는 서비스
settings-provider-current = 사용 중
settings-provider-sign-in = 로그인
settings-profile-account = { $provider } · { $account }
settings-sign-out = 로그아웃
settings-lastfm = Last.fm
settings-lastfm-detail = Sonora는 개인 Last.fm API 계정을 통해 스크로블링합니다. 계정을 만든 후 여기에 키와 시크릿을 붙여넣으세요.
settings-librefm = Libre.fm
settings-listenbrainz = ListenBrainz
settings-listenbrainz-detail = ListenBrainz 설정 페이지에서 사용자 토큰을 복사해 붙여넣으세요. 자체 ListenBrainz 인스턴스나 multi-scrobbler 같은 도구를 실행하는 경우가 아니면 서버 주소는 비워 두세요.
settings-maloja = Maloja
settings-maloja-detail = Sonora가 연결할 Maloja 서버 주소를 입력하고 API 키 중 하나를 붙여넣으세요.
settings-scrobble-provider = { $provider } 재생 내역 스크로블링
settings-scrobble-provider-detail = { $provider }에서 이미 Last.fm으로 직접 스크로블링하고 있다면 재생 횟수가 중복 기록되지 않도록 이 옵션을 끄세요.
settings-scrobble-off = 연결되지 않음
settings-scrobble-on = 연결됨
settings-scrobble-waiting = 브라우저 응답 대기 중…
settings-scrobble-as = { $name }(으)로 스크로블링 중
settings-scrobble-failed = 연결할 수 없음
settings-scrobble-connect = 연결
settings-scrobble-disconnect = 연결 해제
settings-scrobble-title = { $service } 연결
settings-scrobble-request = API 키 발급받기
settings-scrobble-token-request = 토큰 발급받기
settings-scrobble-key = API 키
settings-scrobble-secret = API 시크릿
settings-scrobble-token = 사용자 토큰
settings-scrobble-server = 서버 주소
settings-scrobble-server-optional = 서버 주소 (선택 사항)
settings-local-folder = 음악 폴더
settings-local-folder-empty = 구성되지 않음
settings-choose-folder = 폴더 선택…
settings-add-folder = 폴더 추가
settings-remove-folder = 폴더 제거
settings-rescan = 다시 스캔
settings-scan-walking = 스캔 중…
settings-scan-progress = { $percent }%
settings-scan-done = { $seconds }초 만에 완료됨
settings-version = 버전
settings-version-detail = 현재 실행 중인 Sonora 빌드 버전
settings-license = 라이선스
settings-license-detail = GNU 일반 공중 사용 허가서(GPL) 버전 3 이상
settings-source = 소스 코드
settings-source-detail = 이 빌드에 해당하는 소스 코드
settings-team = 팀
settings-role-lead-maintainer = 수석 관리자
settings-role-maintainer = 관리자
settings-role-contributor = 기여자
settings-notice = Copyright © 2026 Sonora Contributors. Sonora는 어떠한 보증도 제공하지 않습니다. 본 프로그램은 자유 소프트웨어이며 GNU 일반 공중 사용 허가서(GPL) 버전 3 이상의 조건에 따라 재배포할 수 있습니다. Sonora는 비공식 앱이며 Spotify AB와 제휴 관계가 없습니다.

# themes
theme-system = 시스템
theme-dark = 다크
theme-light = 라이트
theme-unavailable = { $name } (사용 불가)
theme-install-title = { $name } 테마를 설치하시겠습니까?
theme-install-detail = 테마 폴더에 추가됩니다.
theme-install-confirm = 설치

# corners
corners-square = 직각
corners-subtle = 약간 둥글게
corners-rounded = 둥글게
corners-round = 매우 둥글게

# motion
motion-system = 시스템 설정 따름
motion-always = 항상 사용
motion-never = 사용 안 함
pace-slow = 느리게
pace-base = 보통
pace-quick = 빠르게
saver-off = 끔
saver-fps = { $fps } FPS

toast-playlist-created = 플레이리스트가 생성되었습니다.
toast-playlist-renamed = 플레이리스트 이름이 변경되었습니다.
toast-playlist-updated = 플레이리스트가 업데이트되었습니다.
toast-playlist-cover-changed = 플레이리스트 커버가 변경되었습니다.
toast-playlist-cover-removed = 플레이리스트 커버가 제거되었습니다.
toast-playlist-deleted = 플레이리스트가 삭제되었습니다.
toast-local-delete-failed = 일부 트랙 파일을 삭제하지 못했습니다.
toast-playlist-added = 플레이리스트가 라이브러리에 추가되었습니다.
toast-playlist-removed = 플레이리스트가 라이브러리에서 제거되었습니다.
toast-playlist-visibility = 플레이리스트 공개 설정이 변경되었습니다.
toast-track-added = { $name }에 추가됨
toast-track-removed = { $name }에서 제거됨
toast-track-moved = { $name }(으)로 이동됨
toast-track-removed-unnamed = 플레이리스트에서 제거됨
toast-playlist-failed = 변경 사항을 저장하지 못했습니다.
toast-playlist-busy = 다른 변경 작업이 아직 진행 중입니다.
toast-playlist-signed-out = 플레이리스트를 변경하려면 로그인하세요.
toast-playlist-imported = { $name } 가져옴
toast-playlist-imported-partly = 라이브러리에 없는 { $count ->
        [one] 1곡을
       *[other] { $count }곡을
    } 제외하고 { $name } 가져옴
toast-playlist-import-empty = { $name }의 곡 중 라이브러리에 있는 곡이 없습니다.
toast-playlist-exported = 플레이리스트를 내보냈습니다.
toast-file-dialog-failed = 파일 대화 상자를 열 수 없습니다.
toast-queued-track = 재생 목록에 { $name } 추가됨
toast-next-track = 바로 다음에 { $name } 재생
toast-last-track = 맨 마지막에 { $name } 재생
toast-queued-album = 재생 목록에 앨범 추가됨
toast-next-album = 바로 다음에 앨범 재생
toast-last-album = 맨 마지막에 앨범 재생
toast-queued-playlist = 재생 목록에 플레이리스트 추가됨
toast-next-playlist = 바로 다음에 플레이리스트 재생
toast-last-playlist = 맨 마지막에 플레이리스트 재생
toast-queued-artist = 재생 목록에 아티스트 추가됨
toast-next-artist = 바로 다음에 아티스트 재생
toast-last-artist = 맨 마지막에 아티스트 재생
toast-queue-failed = 재생 목록에 추가하지 못했습니다.
toast-queue-local-elsewhere = 로컬 파일은 다른 기기의 재생 목록에 추가할 수 없습니다.
toast-queued-tracks = 재생 목록에 { $count }트랙 추가됨
toast-next-tracks = 바로 다음에 { $count }트랙 재생
toast-last-tracks = 맨 마지막에 { $count }트랙 재생
toast-queue-cleared = 재생 목록을 비웠습니다.
toast-queue-removed-track = 재생 목록에서 { $name } 제거됨
toast-queue-removed-tracks = 재생 목록에서 { $count }트랙 제거됨
toast-sleep-ended = 취침 타이머가 종료되었습니다.
toast-radio-failed = { $name }의 라디오를 시작할 수 없습니다.
toast-similar-failed = { $name }와(과) 비슷한 트랙을 찾을 수 없습니다.
toast-open-failed = { $name }을(를) 열 수 없습니다.
toast-open-unsupported = { $name }은(는) Sonora에서 재생할 수 있는 오디오 파일이 아닙니다.
toast-open-all-failed = { $count }개 파일을 열 수 없습니다.
toast-open-some-failed = 일부 파일을 열 수 없습니다.
toast-keys-error = 재생 키 오류
toast-keys-refused = 여기에서는 Spotify가 작동하지 않을 수 있습니다.
toast-why = 이유가 무엇인가요?
toast-soloist-setup = Soloist 설정
toast-soloist-refused = Soloist가 더 이상 로그인할 수 없습니다.
toast-soloist-ready = 이제 Spotify가 Soloist를 통해 재생됩니다.
toast-soloist-remove-failed = Soloist를 제거하지 못했습니다.
toast-sign-in-to-play = { $name }은(는) 로그인한 사용자에게만 스트리밍됩니다.
toast-restore-failed = { $name }에 다시 로그인하지 못해 해당 서비스를 제외하고 계속합니다.
toast-session-invalidated = { $name } 세션이 만료되었습니다.
toast-sign-in = 로그인
toast-track-unplayable = { $name }을(를) 재생할 수 없습니다.
toast-playback-stopped = 여러 트랙을 연속으로 재생할 수 없어 재생이 중지되었습니다.
toast-widevine-failed = 재생 실패
toast-widevine-borrowed = Widevine 업데이트를 시도해 보세요.
toast-open-settings = 설정
toast-throttled = { $name }에서 일시적으로 재생을 제한하고 있습니다. 잠시 후 다시 시도합니다.
toast-still-throttled = { $name }에서 여전히 재생을 제한하고 있습니다. 다시 시도하려면 재생 버튼을 누르세요.
toast-library-add-failed = { $name }을(를) 라이브러리에 추가하지 못했습니다.
toast-library-remove-failed = { $name }을(를) 라이브러리에서 제거하지 못했습니다.
toast-library-added = 라이브러리에 추가됨
toast-library-removed = 라이브러리에서 제거됨
toast-library-removed-one = 라이브러리에서 { $name } 제거됨
toast-library-songs-removed = 라이브러리에서 { $count ->
        [one] 1곡
       *[other] { $count }곡
    } 제거됨
toast-library-albums-removed = 라이브러리에서 앨범 { $count ->
        [one] 1개
       *[other] { $count }개
    } 제거됨
toast-playlists-removed = 라이브러리에서 플레이리스트 { $count ->
        [one] 1개
       *[other] { $count }개
    } 제거됨
toast-favorite-removed = 즐겨찾기에서 { $name } 제거됨
toast-favorite-songs-removed = 즐겨찾기에서 { $count ->
        [one] 1곡
       *[other] { $count }곡
    } 제거됨
toast-favorite-albums-removed = 즐겨찾기에서 앨범 { $count ->
        [one] 1개
       *[other] { $count }개
    } 제거됨
toast-favorite-artists-removed = 즐겨찾기에서 아티스트 { $count ->
        [one] 1명
       *[other] { $count }명
    } 제거됨
toast-tracks-removed = { $name }에서 { $count ->
        [one] 1곡
       *[other] { $count }곡
    } 제거됨
toast-tracks-removed-unnamed = 플레이리스트에서 { $count ->
        [one] 1곡
       *[other] { $count }곡
    } 제거됨
toast-unpinned = { $name } 고정 해제됨

# lyrics
lyrics-title = 가사
lyrics-idle = 가사를 보려면 음악을 재생하세요.
lyrics-loading = 가사 찾는 중…
lyrics-missing = 죄송합니다. 가사를 찾을 수 없습니다.
lyrics-instrumental = 연주곡입니다.
lyrics-failed = 가사 서비스에 연결할 수 없습니다.
lyrics-follow = 곡 진행에 맞춰 가사 따라가기
lyrics-source = 가사 제공: { $source }
lyrics-source-local = 로컬 파일의 가사
lyrics-writers = 작사: { $writers }
lyrics-instrumental-break = 간주 중

update-available = Sonora { $version } 출시됨
update-detail = 현재 { $running } 버전을 사용 중입니다. 변경 사항을 확인하거나 지금 업데이트하세요.
update-detail-notes = 현재 { $running } 버전을 사용 중입니다. 변경 사항을 확인한 후 설치한 방식대로 Sonora를 업데이트하세요.
update-notes = 새로운 기능
update-now = 업데이트
update-working = 업데이트 다운로드 중…
update-failed = 업데이트를 설치하지 못했습니다. 릴리스 페이지에서 다시 시도하세요.
update-never = 업데이트 확인 안 함
notice-never = 다시 표시 안 함
requirement-audio-bridge-title = Sonora가 사운드 서버에 연결할 수 없습니다.
requirement-audio-bridge = { $server ->
    [pulseaudio] PulseAudio가 실행 중이지만 ALSA가 이에 연결되어 있지 않아 소리가 들리지 않습니다. pulseaudio-alsa를 설치하고 Sonora를 다시 시작하세요.
   *[pipewire] PipeWire가 실행 중이지만 ALSA가 이에 연결되어 있지 않아 소리가 들리지 않습니다. pipewire-alsa를 설치하고 Sonora를 다시 시작하세요.
}
requirement-audio-output-title = 사운드 출력 없음
requirement-audio-output = Sonora가 오디오 장치를 열 수 없어 소리가 들리지 않습니다. 장치가 연결되어 있고 다른 앱에서 점유하고 있지 않은지 확인하세요.
requirement-web-engine-title = 브라우저 로그인을 사용할 수 없음
requirement-web-engine = 브라우저 창에서 YouTube Music, Deezer 또는 Apple Music에 로그인하려면 webkit2gtk-4.1을 설치하세요. 설치 전까지는 쿠키를 직접 붙여넣어 로그인할 수 있습니다.
requirement-help = 설치할 항목
settings-check-updates = 업데이트 확인
settings-check-updates-detail = 시작 시 GitHub에서 새 버전 출시 여부를 한 번 확인합니다. 패키지 관리자로 업데이트를 관리하는 경우 변경 내역을 안내하며, 그렇지 않은 경우 Sonora가 직접 업데이트를 설치합니다.
settings-update-channel = 릴리스 채널
settings-update-channel-detail = Stable은 정식 릴리스를 따릅니다. Nightly는 매일 밤 생성되는 main 브랜치 빌드를 따르며, 변경 사항을 가장 먼저 받아볼 수 있지만 테스트가 덜 진행되었을 수 있습니다.
update-channel-stable = Stable
update-channel-nightly = Nightly
settings-log = 로그 파일
settings-log-detail = Sonora 실행 중 기록된 내용입니다. 버그 제보 시 첨부해 주세요.
settings-log-open = 로그 열기

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
toast-tags-saved = { $name }의 태그가 저장되었습니다.
toast-tags-failed = 태그를 저장하지 못했습니다.
toast-output-select-failed = { $name }을(를) 선택할 수 없습니다.
toast-output-default-failed = 시스템 기본 장치를 선택할 수 없습니다.
toast-output-direct-failed = 장치에서 직접 재생할 수 없습니다.
toast-output-rate-failed = 장치의 샘플 레이트를 전환할 수 없습니다.

nav-pin = 고정
toast-library-pin-failed = 고정 상태를 업데이트하지 못했습니다.
toast-jam-unreadable = 참여할 수 있는 Jam 링크가 아닙니다.
toast-jam-needs-connect = Jam에 참여하려면 설정에서 “Spotify 기기로 표시”를 켜세요.
toast-jam-failed = Jam을 시작하거나 참여하지 못했습니다.
toast-jam-ended = Jam이 종료되었습니다.
toast-jam-left = Jam에서 나갔습니다.
toast-jam-leave-failed = Jam을 종료하거나 나가지 못했습니다.
toast-session-needs-provider = 세션을 시작하려면 스트리밍 서비스에 로그인하세요.
toast-session-not-found = 해당 코드와 일치하는 세션이 없습니다.
toast-session-provider = 이 세션은 { $name }에서 재생 중입니다. 참여하려면 { $name }에 로그인하세요.
toast-session-full = 해당 세션의 정원이 가득 찼습니다.
toast-session-rate-limited = 잘못된 코드를 너무 많이 입력했습니다. 몇 분 후에 다시 시도하세요.
toast-session-too-many = 현재 네트워크에서 이미 최대 개수의 세션을 호스팅하고 있습니다. 새 세션을 시작하려면 기존 세션을 종료하세요.
toast-session-unreachable = Sonora 세션 서버에 연결할 수 없습니다. 나중에 다시 시도하세요.
toast-session-failed = 세션을 시작하거나 참여하지 못했습니다.
toast-session-lost = 세션과의 연결이 끊어졌습니다.
toast-session-ended = 세션이 종료되었습니다.
toast-session-left = 세션에서 나갔습니다.
toast-session-kicked = 호스트가 나를 세션에서 내보냈습니다.
toast-session-code-copied = 코드가 복사되었습니다.
toast-sessions-server-saved = 세션 서버가 저장되었습니다.
toast-sessions-server-empty = 먼저 서버 주소를 입력하세요.
toast-sessions-server-malformed = 올바른 서버 주소 형식이 아닙니다.
toast-sessions-server-silent = 이 주소에서 응답하는 Sonora 세션 서버가 없어 저장되지 않았습니다.
nav-nothing-pinned = 고정된 항목 없음
nav-pins-alphabetical = 가나다순
nav-pins-kind = 유형별
nav-show-full-library = 전체 라이브러리 표시
nav-return-top = 맨 위로 돌아가기

# trouble
trouble-offline = 연결 없음
trouble-offline-detail = 인터넷 연결을 확인하고 다시 시도하세요.
trouble-not-loaded = 불러올 수 없음
trouble-retry = 다시 시도
toast-offline = 인터넷에 연결되어 있지 않습니다. 다시 연결될 때까지 스트리밍되지 않습니다.
toast-offline-unsupported = { $name }은(는) 온라인 상태일 때만 다운로드 항목을 재생합니다.
toast-offline-mode = 오프라인 상태입니다. 이 곡을 스트리밍하려면 온라인으로 전환하세요.
offline-title = 오프라인 상태입니다.
offline-go-online = 온라인으로 전환
toast-download-failed = 다운로드할 트랙 목록을 불러오지 못했습니다.
toast-percent = { $percent }%
toast-download-started = { $name } 다운로드 시작됨
toast-downloads-started = { $count ->
    [one] { $count }곡 다운로드 시작됨
   *[other] { $count }곡 다운로드 시작됨
}
toast-download-cancelled = { $name } 다운로드 취소됨
toast-downloads-cancelled = { $count ->
    [one] { $count }트랙 다운로드 취소됨
   *[other] { $count }트랙 다운로드 취소됨
}
toast-settings-broken = 변경 사항을 저장하려면 settings.json의 { $name }번째 줄을 수정하세요.
toast-tray-unavailable = 아이콘을 표시할 시스템 트레이가 없습니다.
toast-undo = 실행 취소
toast-link-copied = 링크가 복사되었습니다.
toast-link-sign-in = 이 링크를 열려면 { $name }에 로그인하세요.
toast-keybind-taken = { $name }에서 해당 단축키가 해제되었습니다.
toast-link-unreadable = Sonora에서 열 수 있는 경로가 아닌 링크입니다.
toast-theme-link-unreadable = 테마 링크가 손상되었거나 테마가 아닌 다른 데이터가 포함되어 있습니다.
toast-theme-install-failed = { $name }을(를) 설치할 수 없습니다.
toast-theme-delete-failed = { $name }을(를) 삭제할 수 없습니다.
toast-code-copied = 코드가 복사되었습니다.
menu-copy-link-none = 공유할 링크가 없습니다.
menu-copy-link-local = 로컬 아티스트와 플레이리스트에는 링크가 없습니다.
toast-settings-save-failed = 설정을 저장하지 못했습니다.
toast-settings-overwritten = 디스크에서 settings.json이 변경되어 마지막 변경 사항이 저장되지 않았습니다.
toast-widevine-download-failed = Widevine을 다운로드하지 못했습니다.
toast-widevine-install-failed = Widevine을 설치하지 못했습니다.
toast-widevine-remove-failed = Widevine을 제거하지 못했습니다.
toast-library-scanned = { $count ->
    [one] 라이브러리 스캔 완료, 1곡을 찾았습니다.
   *[other] 라이브러리 스캔 완료, { $count }곡을 찾았습니다.
}
toast-scan-failed = 음악 폴더를 스캔하지 못했습니다.
toast-folder-removed = { $name } 제거됨
toast-folder-missing = 디스크에 { $name }이(가) 더 이상 존재하지 않습니다.
toast-history-removed = { $count ->
    [one] 재생 기록에서 제거됨
   *[other] 재생 기록에서 { $count }곡 제거됨
}
toast-history-remove-failed = 재생 기록에서 곡을 제거하지 못했습니다.
toast-history-restore-failed = 재생 기록을 복원하지 못했습니다.
toast-history-clear-failed = 재생 기록을 지우지 못했습니다.

# power
wake-reason = 음악 재생 중

toast-sample-plain = 이벤트가 생성되었습니다.
toast-sample-done = 라이브러리에 저장되었습니다.
toast-sample-info = 이벤트 시작 10분 전에 도착해 주세요.
toast-sample-warning = 세션이 5분 후에 종료됩니다.
toast-sample-failed = 서버에 연결할 수 없습니다.
toast-sample-loading = 트랙 업로드 중
toast-sample-event = 이벤트 생성됨
toast-sample-event-detail = 12월 3일 일요일 오전 9:00
toast-sample-delete = { $name } 삭제됨
toast-sample-undo = 실행 취소
toast-sample-cancel = 취소
toast-sample-undone = 실행 취소됨
toast-sample-queued = 재생 목록에 { $name } 추가됨
toast-sample-long = 이 알림 메시지는 다른 메시지보다 훨씬 길기 때문에 닫기 버튼을 카드 밖으로 밀어내지 않고 두 번째 줄과 세 번째 줄로 자연스럽게 줄바꿈되어야 합니다.
toast-sample-saving = { $name } 저장 중
toast-sample-saved = { $name } 저장됨
toast-sample-save-failed = { $name }을(를) 저장하지 못했습니다.
toast-sample-save-failed-detail = 서버에서 503 Service Unavailable 응답을 보냈습니다.
toast-sample-lasting = 이 알림은 직접 닫을 때까지 유지됩니다.
toast-sample-burst = 알림 번호 { $name }

# Frequency response graph
response-graph-hz = { $value } Hz
response-graph-khz = { $value } kHz
response-graph-db = { $value } dB
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB