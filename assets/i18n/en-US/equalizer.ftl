equalizer-advanced = Advanced
equalizer-simple = Simple
equalizer-sound = Sound
equalizer-profile-default = Default
equalizer-profile-new = Profile { $number }
equalizer-profile-pasted = Pasted profile
equalizer-profile-name-hint = Profile name
equalizer-save-as = Save as profile
equalizer-rename = Rename profile
equalizer-delete = Delete profile
equalizer-bass = Bass
equalizer-treble = Treble
equalizer-tone-reset = Reset to 0 dB
equalizer-preamp = Preamp
equalizer-preamp-auto = Automatic
equalizer-preamp-manual = Manual
equalizer-decibels = dB
equalizer-hertz = Hz
equalizer-q = Q
equalizer-octaves = oct
equalizer-no-gain = —
equalizer-filters = Filters
equalizer-no-filters = No filters yet. Add one, pick a preset above, or import a profile.
equalizer-add-filter = Add filter
equalizer-remove-filter = Remove filter
equalizer-filter = Filter { $number }
equalizer-filter-kind = Filter { $number } type
equalizer-kind-peak = Peak
equalizer-kind-low-shelf = Low shelf
equalizer-kind-high-shelf = High shelf
equalizer-kind-low-pass = Low pass
equalizer-kind-high-pass = High pass
equalizer-kind-notch = Notch
equalizer-kind-band-pass = Band pass
equalizer-kind-all-pass = All pass
equalizer-transfer = Import and export
equalizer-import = Import
equalizer-export = Export
equalizer-import-file = Import file…
equalizer-export-file = Export file…
equalizer-imported = Imported { $name }
equalizer-imported-skipped = Imported { $name } without { $count ->
        [one] one line
       *[other] { $count } lines
    }
equalizer-import-nothing = Found no equalizer settings to import
equalizer-import-too-many = Not imported: { $count ->
        [one] one filter is on
       *[other] { $count } filters are on
    }, more than a profile holds
equalizer-import-too-large = Not imported: the text is over 1 MB, too large for an equalizer profile
equalizer-filter-limit =A profile holds up to { $max } filters.
equalizer-paste-empty = The clipboard holds no text
equalizer-copied = Copied the profile as Equalizer APO text
equalizer-exported = Exported the profile
equalizer-file-unreadable = Could not read the file
equalizer-file-unwritable = Could not write the file
equalizer-value-refused = Kept the previous value
equalizer-skipped-line = Line { $line }: { $reason }
equalizer-skipped-more = and { $count ->
        [one] one more
       *[other] { $count } more
    }
equalizer-skip-unknown-type = unknown filter type { $name }
equalizer-skip-unreadable = not a filter Sonora can read
equalizer-skip-no-frequency = no frequency
equalizer-skip-no-gain = no gain
equalizer-skip-no-width = no Q or bandwidth
equalizer-skip-expression = a value written as an expression
equalizer-skip-no-preamp = the preamp is not a number
equalizer-skip-unsupported = { $name } is not supported
equalizer-rules = Device rules
equalizer-rule-other = Any other device
equalizer-rule-playing = Playing now
equalizer-rule-replaces = Replaces its rule
equalizer-rule-choose-device = Choose a device
equalizer-mode = Mode
equalizer-preamp-level = Preamp level
equalizer-no-gain-spoken = No gain
equalizer-filter-frequency = Filter { $number } frequency
equalizer-filter-gain = Filter { $number } gain
equalizer-filter-width = Filter { $number } width
equalizer-graph = Frequency response
equalizer-graph-detail = { $count ->
    [0] No filters
    [one] 1 filter
   *[other] { $count } filters
    }, preamp at { $db } dB
equalizer-rule-device = Device
equalizer-rule-choose-sound = Choose a sound
equalizer-rule-add = Add rule
equalizer-rule-remove = Remove rule
equalizer-output-port = { $device } · { $port }
equalizer-invalid-frequency = Frequency must be between { $low } and { $high } Hz
equalizer-invalid-gain = Gain must be between { $low } and { $high } dB
equalizer-invalid-q = Q must be between { $low } and { $high }
equalizer-invalid-octaves = Bandwidth must be between { $low } and { $high } octaves
equalizer-invalid-shelf-octaves = A shelf takes a Q, not a bandwidth
equalizer-invalid-preamp = Preamp must be between { $low } and { $high } dB
equalizer-overload = Clipping by { $db } dB
equalizer-overload-slight = Clipping by under 0.1 dB
equalizer-lower-preamp = Lower preamp
equalizer-level = Level
equalizer-status-bit-perfect = Bit-perfect
equalizer-status-bit-perfect-detail = Bit-perfect from Sonora
equalizer-status-server = The sound server then converts it to { $khz } kHz
equalizer-status-altered = Not bit-perfect due to:
equalizer-status-separator = {", "}
equalizer-status-equalized = EQ
equalizer-status-volume = Volume
equalizer-status-normalized = Normalized
equalizer-status-direct = Played by Soloist, past Sonora
equalizer-status-rate = { $from } → { $to } kHz
equalizer-status-bits = { $from } → { $to } bit
equalizer-status-float = Float → { $to } bit
equalizer-status-channels = { $from ->
    [1] Mono
    [2] Stereo
   *[other] { $from } ch
} → { $to ->
    [1] mono
    [2] stereo
   *[other] { $to } ch
}

# Frequency response graph
response-graph-readout = { $frequency } · { $direction ->
        [up] +{ $gain }
        [down] -{ $gain }
       *[flat] { $gain }
    } dB
