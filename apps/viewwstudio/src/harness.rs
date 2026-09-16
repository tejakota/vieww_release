//! The Gradle and Xcode projects a scaffolded app needs to become an APK or an
//! `.ipa`.
//!
//! # Why this file exists
//!
//! [`export`](crate::export) has always planned honest Android and iOS builds:
//! `cargo ndk` then Gradle for a debug-signed APK, `xcodebuild` for a simulator
//! `.app` or a signed `.ipa`. Those plans run their second step in
//! `<project>/android` and `<project>/ios` — and [`scaffold`](crate::scaffold)
//! produced neither. A new project was `Cargo.toml`, `src/main.rs` and a
//! screens module: a desktop winit application. So the studio could export an
//! app whose harnesses somebody else had written, and could not take one of its
//! own projects to a phone.
//!
//! This closes that loop. The paths here are not chosen freshly; they are the
//! paths `export` already runs in, and `tests/scaffold_exports.rs` asserts that
//! every file an export plan reaches for is one this module writes.
//!
//! # One Rust crate, three shapes
//!
//! A desktop app is a binary with a `main`. Android does not call `main` — the
//! activity loads a shared library and calls `android_main` inside it — so the
//! entry point has to be a `cdylib`. iOS does call a `main`, but it must be
//! **Objective-C's**, because `UIApplicationMain` has to own the process before
//! any window exists; the Rust is linked in as a `staticlib` and called from
//! it.
//!
//! Hence a scaffolded project is a library with all three crate types and a
//! thin `src/main.rs` over it, rather than three projects that drift apart.
//! `src/lib.rs` holds the one `run` the three entry points share.
//!
//! # What is generated, and what is deliberately not
//!
//! **Not the Gradle wrapper JAR.** `gradlew` is a shell script and a *binary*
//! `gradle-wrapper.jar`, and a scaffolder that writes source files has no
//! business emitting a JAR — so `gradle-wrapper.properties` is written, the
//! JAR is not, and `export::android` runs `./gradlew` when it is there and
//! `gradle` when it is not. Running `gradle wrapper` once in `android/` turns
//! the second into the first.
//!
//! **No signing configuration beyond the defaults.** A debug APK is
//! debug-signed by Gradle with the key every Android SDK ships, which is what
//! "send this to a colleague" needs. Release signing wants a keystore and two
//! passwords, which are not things to put in a generated file.
//!
//! # These files have not been run
//!
//! No Gradle and no Xcode exist on the machine this was written on, so what is
//! checked here is what can be: the XML and property lists parse, the project
//! file's structure is walked and its cross-references resolved, and every path
//! the export plans name is a path this module writes. What has not been
//! checked is that Gradle and Xcode like them. That is stated here rather than
//! in a commit message because it is the first thing somebody debugging a
//! failed first build needs to know.

use std::path::PathBuf;

/// A crate name as Rust will see it: `my-app` is `my_app` to the linker.
///
/// The Android manifest's `android.app.lib_name` and the iOS `-l` flag both
/// name the *library*, not the package, and getting that wrong produces
/// `dlopen failed: library "libmy-app.so" not found` at launch rather than
/// anything at build time.
#[must_use]
pub fn lib_name(name: &str) -> String {
    name.replace('-', "_")
}

/// The placeholder application id.
///
/// `com.example` is reserved for exactly this and is refused by the Play
/// Store, which is the point: a generated identifier should be impossible to
/// ship by accident.
#[must_use]
pub fn bundle_id(name: &str) -> String {
    format!("com.example.{}", lib_name(name))
}

/// Every Android and iOS file a new project starts with.
#[must_use]
pub fn files(name: &str) -> Vec<(PathBuf, String)> {
    let mut out = android(name);
    out.extend(ios(name));
    out
}

// ---------------------------------------------------------------------------
// Android
// ---------------------------------------------------------------------------

/// The Gradle project, laid out the way `export::android` runs it.
fn android(name: &str) -> Vec<(PathBuf, String)> {
    let lib = lib_name(name);
    let id = bundle_id(name);
    vec![
        (
            PathBuf::from("android/settings.gradle"),
            format!(
                r#"// Repositories are declared here rather than in each module, which is what
// Gradle 7+ expects and what `dependencyResolutionManagement` enforces.
pluginManagement {{
    repositories {{
        google()
        mavenCentral()
        gradlePluginPortal()
    }}
}}

dependencyResolutionManagement {{
    repositories {{
        google()
        mavenCentral()
    }}
}}

rootProject.name = "{name}"
include ":app"
"#
            ),
        ),
        (
            PathBuf::from("android/build.gradle"),
            r#"// The plugin is declared here and applied in `app/build.gradle`, so the
// version lives in one place.
plugins {
    id "com.android.application" version "8.5.2" apply false
}
"#
            .to_string(),
        ),
        (
            PathBuf::from("android/gradle.properties"),
            r#"# Gradle's default heap is not enough for the Android plugin on a large
# project, and the failure is an OutOfMemoryError in a daemon rather than
# anything that names a cause.
org.gradle.jvmargs=-Xmx2048m

# Required by AGP 8. No AndroidX code is generated here, but the plugin reads
# the flag whether or not anything uses it.
android.useAndroidX=true
"#
            .to_string(),
        ),
        (
            PathBuf::from("android/gradle/wrapper/gradle-wrapper.properties"),
            r#"# The wrapper's *properties*, without the wrapper's JAR — a scaffolder that
# writes source files has no business emitting a binary. Run `gradle wrapper`
# in this directory once and `gradlew` appears beside it; until then the studio
# invokes `gradle` directly, which does the same work.
distributionBase=GRADLE_USER_HOME
distributionPath=wrapper/dists
distributionUrl=https\://services.gradle.org/distributions/gradle-8.7-bin.zip
networkTimeout=10000
validateDistributionUrl=true
zipStoreBase=GRADLE_USER_HOME
zipStorePath=wrapper/dists
"#
            .to_string(),
        ),
        (
            PathBuf::from("android/app/build.gradle"),
            format!(
                r#"plugins {{
    id "com.android.application"
}}

android {{
    namespace "{id}"
    compileSdk 34

    defaultConfig {{
        applicationId "{id}"
        // 24 is the floor the framework's own NDK linker and device scripts
        // already target, so the APK and the Rust are built against one.
        minSdk 24
        targetSdk 34
        versionCode 1
        versionName "0.1.0"

        ndk {{
            // One architecture, matching `cargo ndk -t arm64-v8a` in the export
            // plan. Every phone shipped since 2017 is arm64; adding armeabi-v7a
            // here without adding it there produces an APK whose 32-bit slice is
            // empty, which fails at launch rather than at build.
            abiFilters "arm64-v8a"
        }}
    }}

    sourceSets {{
        main {{
            // **Where `cargo ndk -o target/android/jniLibs` puts the library.**
            // Two levels up because this file is `android/app/`, and the Rust
            // target directory belongs to the crate at the project root.
            jniLibs.srcDirs = ["../../target/android/jniLibs"]
        }}
    }}

    packaging {{
        jniLibs {{
            // `NativeActivity` resolves `android.app.lib_name` through
            // `System.loadLibrary`, which wants a real file on disk. Modern AGP
            // defaults to leaving `.so` files compressed inside the APK, and the
            // symptom is `dlopen failed: library "lib{lib}.so" not found` on a
            // device while every build step reported success.
            useLegacyPackaging true
        }}
    }}

    buildTypes {{
        debug {{
            // Debug-signed with the key every Android SDK ships, which is what
            // makes `adb install` work without a keystore.
            debuggable true
        }}
        release {{
            minifyEnabled false
            // Deliberately unsigned: a release key is a keystore and two
            // passwords, and neither belongs in a generated file. Configure a
            // `signingConfig` here when you have somewhere safe to keep them.
        }}
    }}
}}
"#
            ),
        ),
        (
            PathBuf::from("android/app/src/main/AndroidManifest.xml"),
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<!--
  No Java or Kotlin anywhere in this project, which `android:hasCode="false"`
  states: the activity is the platform's own `NativeActivity` and the whole
  application is the Rust library it loads.
-->
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

    <!-- vello renders through Vulkan or GLES 3; a device with neither cannot
         run this, and saying so here keeps it out of the Play Store listing for
         those devices rather than crashing on them. -->
    <uses-feature
        android:glEsVersion="0x00030000"
        android:required="true" />

    <application
        android:label="{name}"
        android:hasCode="false"
        android:extractNativeLibs="true"
        android:theme="@android:style/Theme.DeviceDefault.NoActionBar">

        <activity
            android:name="android.app.NativeActivity"
            android:exported="true"
            android:screenOrientation="portrait"
            android:configChanges="orientation|screenSize|smallestScreenSize|keyboardHidden|screenLayout|density">

            <!-- The name of the `cdylib` in `Cargo.toml`, without `lib` or
                 `.so`. If this and the crate's library name disagree the app
                 installs cleanly and dies on launch. -->
            <meta-data
                android:name="android.app.lib_name"
                android:value="{lib}" />

            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
    </application>
</manifest>
"#
            ),
        ),
        (
            PathBuf::from("android/README.md"),
            format!(
                r#"# Android

Built in two steps, which is what vieww Studio's Export view runs for you:

```console
cargo ndk -t arm64-v8a -o target/android/jniLibs build --release
cd android && ./gradlew assembleDebug     # or: gradle assembleDebug
```

The APK lands at `android/app/build/outputs/apk/debug/app-debug.apk`.

## The wrapper

There is no `gradlew` here yet, because it needs a binary JAR that a source
scaffolder should not be writing. Run this once:

```console
cd android && gradle wrapper
```

Until you do, the studio invokes `gradle` directly and everything works the
same; afterwards the build is pinned to the version in
`gradle/wrapper/gradle-wrapper.properties`.

## Before you publish

* `applicationId` is `{id}`. `com.example` is reserved and the Play Store
  refuses it — change it in `app/build.gradle`, and change `namespace` with it.
* The release build type is unsigned on purpose. Add a `signingConfig` once you
  have a keystore somewhere safe.
"#
            ),
        ),
    ]
}

// ---------------------------------------------------------------------------
// iOS
// ---------------------------------------------------------------------------

/// The Xcode project, laid out the way `export::ios_app` and `ios_ipa` run it.
fn ios(name: &str) -> Vec<(PathBuf, String)> {
    let lib = lib_name(name);
    let id = bundle_id(name);
    let project = format!("ios/{name}.xcodeproj");
    vec![
        (
            PathBuf::from(format!("{project}/project.pbxproj")),
            pbxproj(name),
        ),
        (
            PathBuf::from(format!("{project}/xcshareddata/xcschemes/{name}.xcscheme")),
            xcscheme(name),
        ),
        (
            PathBuf::from("ios/App/main.m"),
            r#"// The process's entry point, and why it is Objective-C.
//
// `UIApplicationMain` has to own the process before any window exists, and it
// never returns — so on iOS the `main` that runs is not Rust's. The Rust side
// is linked in as a static library and exposes one symbol; this calls it, and
// `vieww_ios_main` hands control to winit, which calls `UIApplicationMain`
// itself.
//
// Declared here rather than in a generated header because it is one function
// with no arguments, and a header for it would be a file to keep in sync with
// `src/lib.rs` for no benefit.

extern void vieww_ios_main(void);

int main(int argc, char *argv[]) {
    (void)argc;
    (void)argv;
    vieww_ios_main();
    return 0;
}
"#
            .to_string(),
        ),
        (
            PathBuf::from("ios/App/Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDevelopmentRegion</key>
	<string>en</string>
	<key>CFBundleExecutable</key>
	<string>$(EXECUTABLE_NAME)</string>
	<key>CFBundleIdentifier</key>
	<string>$(PRODUCT_BUNDLE_IDENTIFIER)</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>CFBundleName</key>
	<string>{name}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>0.1.0</string>
	<key>CFBundleVersion</key>
	<string>1</string>
	<!-- An app with no launch screen is letterboxed into an iPhone 4 sized box
	     on every modern device. An empty dictionary asks for the system's
	     default one, which is what an app with nothing to show at launch
	     wants. -->
	<key>UILaunchScreen</key>
	<dict/>
	<key>UIRequiredDeviceCapabilities</key>
	<array>
		<string>arm64</string>
	</array>
	<key>UISupportedInterfaceOrientations</key>
	<array>
		<string>UIInterfaceOrientationPortrait</string>
	</array>
</dict>
</plist>
"#
            ),
        ),
        (
            PathBuf::from("ios/ExportOptions.plist"),
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<!--
  Read by `xcodebuild -exportArchive`, which is the last step of the .ipa
  export. `development` produces a build for devices registered to your team —
  the one an ad-hoc tester can install. Change it to `app-store` when you are
  submitting, and add a `teamID` if your Apple ID belongs to more than one team,
  because Xcode cannot guess between two.
-->
<plist version="1.0">
<dict>
	<key>method</key>
	<string>development</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>stripSwiftSymbols</key>
	<true/>
	<key>uploadSymbols</key>
	<true/>
</dict>
</plist>
"#
            .to_string(),
        ),
        (
            PathBuf::from("ios/README.md"),
            format!(
                r#"# iOS

Built in two steps, which is what vieww Studio's Export view runs for you:

```console
cargo build --release --target aarch64-apple-ios-sim   # or aarch64-apple-ios
cd ios && xcodebuild -scheme {name} -sdk iphonesimulator -configuration Release build
```

macOS only. That is Apple's rule rather than a gap in the studio, and the
Export view says so on other hosts instead of offering a build that cannot run.

## How the Rust gets in

The crate builds as a `staticlib` as well as a binary. Xcode links
`lib{lib}.a` out of `../target/$(RUST_TARGET)/release`, where `RUST_TARGET` is
set per SDK in the project's build settings — `aarch64-apple-ios` for a device
and `aarch64-apple-ios-sim` for the simulator. `App/main.m` calls the one symbol
the library exports.

If the linker cannot find `-l{lib}`, the Rust for *that* SDK's target has not
been built yet; the Export view runs both steps in the right order.

## Before you publish

* `PRODUCT_BUNDLE_IDENTIFIER` is `{id}`, which is a placeholder. Change it in
  the target's build settings.
* Signing is automatic, so Xcode uses whichever identity your Apple ID provides.
  There is no identity in this file and none should be added to it.
"#
            ),
        ),
    ]
}

/// A stable 96-bit identifier, as Xcode writes them.
///
/// # Why these are derived rather than random
///
/// A `.pbxproj` is a graph of objects that reference each other by identifier,
/// and Xcode generates them randomly. A scaffolder must not: regenerating a
/// project would then produce a file that differs from the last one in every
/// line, which is unreviewable in a diff and unmergeable in a repository.
///
/// FNV-1a over the project name and the object's role, which is the same hash
/// [`crate::recovery`] uses to name a file — deterministic, no dependency, and
/// nobody has to care that it is not cryptographic because nothing here is a
/// secret. Two different roles collide only if their hashes do, and the roles
/// are a fixed list of eleven strings checked by a test.
fn oid(name: &str, role: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in name.bytes().chain(b"/".iter().copied()).chain(role.bytes()) {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // 24 uppercase hex characters is the shape Xcode writes. The low 32 bits
    // are folded back in to fill the width rather than padding with zeros,
    // which would make every identifier look like the same one at a glance.
    let tail = (hash ^ hash.rotate_left(17)) as u32;
    format!("{hash:016X}{tail:08X}")
}

/// Every object identifier this project uses, by role.
#[cfg(test)]
const ROLES: [&str; 17] = [
    "project",
    "target",
    "product",
    "main.m",
    "main.m build",
    "Info.plist",
    "group root",
    "group app",
    "group products",
    "phase sources",
    "phase frameworks",
    "cfglist project",
    "cfglist target",
    "cfg project debug",
    "cfg project release",
    "cfg target debug",
    "cfg target release",
];

#[expect(
    clippy::too_many_lines,
    reason = "a project file is one literal; splitting it would hide its shape"
)]
fn pbxproj(name: &str) -> String {
    let lib = lib_name(name);
    let id = bundle_id(name);
    let o = |role: &str| oid(name, role);
    let (project, target, product) = (o("project"), o("target"), o("product"));
    let (main_ref, main_build) = (o("main.m"), o("main.m build"));
    let plist_ref = o("Info.plist");
    let (root, app, products) = (o("group root"), o("group app"), o("group products"));
    let (sources, frameworks) = (o("phase sources"), o("phase frameworks"));
    let (cfg_proj, cfg_target) = (o("cfglist project"), o("cfglist target"));
    let (proj_debug, proj_release) = (o("cfg project debug"), o("cfg project release"));
    let (tgt_debug, tgt_release) = (o("cfg target debug"), o("cfg target release"));

    // The settings that decide whether this links, kept in one place because
    // they are what somebody debugging a link failure will come to read.
    let linking = format!(
        r#"				"RUST_TARGET[sdk=iphoneos*]" = "aarch64-apple-ios";
				"RUST_TARGET[sdk=iphonesimulator*]" = "aarch64-apple-ios-sim";
				LIBRARY_SEARCH_PATHS = (
					"$(inherited)",
					"$(PROJECT_DIR)/../target/$(RUST_TARGET)/release",
				);
				OTHER_LDFLAGS = (
					"-l{lib}",
					"-lc++",
					"-framework",
					UIKit,
					"-framework",
					Foundation,
					"-framework",
					Metal,
					"-framework",
					QuartzCore,
					"-framework",
					CoreGraphics,
				);"#
    );

    // Identical in Debug and Release: everything that differs between the two
    // — optimisation, active architecture, product validation — is a *project*
    // level setting above, and duplicating them here would only create two
    // places to change one thing.
    let target_settings = || {
        format!(
            r#"				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				GENERATE_INFOPLIST_FILE = NO;
				INFOPLIST_FILE = App/Info.plist;
				IPHONEOS_DEPLOYMENT_TARGET = 13.0;
				MARKETING_VERSION = 0.1.0;
				PRODUCT_BUNDLE_IDENTIFIER = "{id}";
				PRODUCT_NAME = "$(TARGET_NAME)";
{linking}
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2";"#
        )
    };

    format!(
        r#"// !$*UTF8*$!
{{
	archiveVersion = 1;
	classes = {{
	}};
	objectVersion = 56;
	objects = {{

/* Begin PBXBuildFile section */
		{main_build} /* main.m in Sources */ = {{isa = PBXBuildFile; fileRef = {main_ref} /* main.m */; }};
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		{product} /* {name}.app */ = {{isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = "{name}.app"; sourceTree = BUILT_PRODUCTS_DIR; }};
		{main_ref} /* main.m */ = {{isa = PBXFileReference; lastKnownFileType = sourcecode.c.objc; path = main.m; sourceTree = "<group>"; }};
		{plist_ref} /* Info.plist */ = {{isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = Info.plist; sourceTree = "<group>"; }};
/* End PBXFileReference section */

/* Begin PBXFrameworksBuildPhase section */
		{frameworks} /* Frameworks */ = {{
			isa = PBXFrameworksBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		}};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
		{root} = {{
			isa = PBXGroup;
			children = (
				{app} /* App */,
				{products} /* Products */,
			);
			sourceTree = "<group>";
		}};
		{app} /* App */ = {{
			isa = PBXGroup;
			children = (
				{main_ref} /* main.m */,
				{plist_ref} /* Info.plist */,
			);
			path = App;
			sourceTree = "<group>";
		}};
		{products} /* Products */ = {{
			isa = PBXGroup;
			children = (
				{product} /* {name}.app */,
			);
			name = Products;
			sourceTree = "<group>";
		}};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
		{target} /* {name} */ = {{
			isa = PBXNativeTarget;
			buildConfigurationList = {cfg_target} /* Build configuration list for PBXNativeTarget "{name}" */;
			buildPhases = (
				{sources} /* Sources */,
				{frameworks} /* Frameworks */,
			);
			buildRules = (
			);
			dependencies = (
			);
			name = "{name}";
			productName = "{name}";
			productReference = {product} /* {name}.app */;
			productType = "com.apple.product-type.application";
		}};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
		{project} /* Project object */ = {{
			isa = PBXProject;
			attributes = {{
				BuildIndependentTargetsInParallel = 1;
				LastUpgradeCheck = 1500;
				TargetAttributes = {{
					{target} = {{
						CreatedOnToolsVersion = 15.0;
					}};
				}};
			}};
			buildConfigurationList = {cfg_proj} /* Build configuration list for PBXProject "{name}" */;
			compatibilityVersion = "Xcode 14.0";
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (
				en,
				Base,
			);
			mainGroup = {root};
			productRefGroup = {products} /* Products */;
			projectDirPath = "";
			projectRoot = "";
			targets = (
				{target} /* {name} */,
			);
		}};
/* End PBXProject section */

/* Begin PBXSourcesBuildPhase section */
		{sources} /* Sources */ = {{
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				{main_build} /* main.m in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		}};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
		{proj_debug} /* Debug */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ALWAYS_SEARCH_USER_PATHS = NO;
				ARCHS = arm64;
				CLANG_ENABLE_OBJC_ARC = YES;
				ENABLE_BITCODE = NO;
				GCC_OPTIMIZATION_LEVEL = 0;
				ONLY_ACTIVE_ARCH = YES;
				SDKROOT = iphoneos;
			}};
			name = Debug;
		}};
		{proj_release} /* Release */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
				ALWAYS_SEARCH_USER_PATHS = NO;
				ARCHS = arm64;
				CLANG_ENABLE_OBJC_ARC = YES;
				ENABLE_BITCODE = NO;
				SDKROOT = iphoneos;
				VALIDATE_PRODUCT = YES;
			}};
			name = Release;
		}};
		{tgt_debug} /* Debug */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
{debug_settings}
			}};
			name = Debug;
		}};
		{tgt_release} /* Release */ = {{
			isa = XCBuildConfiguration;
			buildSettings = {{
{release_settings}
			}};
			name = Release;
		}};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
		{cfg_proj} /* Build configuration list for PBXProject "{name}" */ = {{
			isa = XCConfigurationList;
			buildConfigurations = (
				{proj_debug} /* Debug */,
				{proj_release} /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		}};
		{cfg_target} /* Build configuration list for PBXNativeTarget "{name}" */ = {{
			isa = XCConfigurationList;
			buildConfigurations = (
				{tgt_debug} /* Debug */,
				{tgt_release} /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		}};
/* End XCConfigurationList section */
	}};
	rootObject = {project} /* Project object */;
}}
"#,
        debug_settings = target_settings(),
        release_settings = target_settings(),
    )
}

/// The shared scheme, because `xcodebuild -scheme` cannot find an unshared one.
///
/// A scheme Xcode creates for you lives in `xcuserdata` and is per-user, so a
/// project checked out on another machine — or built by the studio, which is
/// another process — has no scheme at all and `xcodebuild` fails with
/// "scheme not found" while the project is plainly there. Writing it into
/// `xcshareddata` is what makes the export plan's `-scheme {name}` resolve.
fn xcscheme(name: &str) -> String {
    let target = oid(name, "target");
    let project = format!("{name}.xcodeproj");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Scheme
   LastUpgradeVersion = "1500"
   version = "1.7">
   <BuildAction
      parallelizeBuildables = "YES"
      buildImplicitDependencies = "YES">
      <BuildActionEntries>
         <BuildActionEntry
            buildForTesting = "YES"
            buildForRunning = "YES"
            buildForProfiling = "YES"
            buildForArchiving = "YES"
            buildForAnalyzing = "YES">
            <BuildableReference
               BuildableIdentifier = "primary"
               BlueprintIdentifier = "{target}"
               BuildableName = "{name}.app"
               BlueprintName = "{name}"
               ReferencedContainer = "container:{project}">
            </BuildableReference>
         </BuildActionEntry>
      </BuildActionEntries>
   </BuildAction>
   <LaunchAction
      buildConfiguration = "Release"
      selectedDebuggerIdentifier = "Xcode.DebuggerFoundation.Debugger.LLDB"
      selectedLauncherIdentifier = "Xcode.DebuggerFoundation.Launcher.LLDB"
      launchStyle = "0"
      useCustomWorkingDirectory = "NO"
      ignoresPersistentStateOnLaunch = "NO"
      debugDocumentVersioning = "YES"
      debugServiceExtension = "internal"
      allowLocationSimulation = "YES">
      <BuildableProductRunnable
         runnableDebuggingMode = "0">
         <BuildableReference
            BuildableIdentifier = "primary"
            BlueprintIdentifier = "{target}"
            BuildableName = "{name}.app"
            BlueprintName = "{name}"
            ReferencedContainer = "container:{project}">
         </BuildableReference>
      </BuildableProductRunnable>
   </LaunchAction>
   <ArchiveAction
      buildConfiguration = "Release"
      revealArchiveInOrganizer = "YES">
   </ArchiveAction>
</Scheme>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hyphenated_name_becomes_a_linkable_library_name() {
        // `my-app` is `libmy_app.so` and `libmy_app.a`, and the manifest and
        // the linker flag both have to say so.
        assert_eq!(lib_name("my-app"), "my_app");
        let files = files("my-app");
        let manifest = find(&files, "android/app/src/main/AndroidManifest.xml");
        assert!(
            manifest.contains(r#"android:value="my_app""#),
            "the activity loads lib<name>.so: {manifest}"
        );
        let project = find(&files, "ios/my-app.xcodeproj/project.pbxproj");
        assert!(
            project.contains("\"-lmy_app\""),
            "and the linker wants the same"
        );
    }

    #[test]
    fn every_object_identifier_is_distinct() {
        // A `.pbxproj` is a graph keyed by these. Two roles sharing one turns a
        // build phase into a file reference, and Xcode's error for that names
        // neither.
        let mut seen: Vec<String> = ROLES.iter().map(|role| oid("app", role)).collect();
        let before = seen.len();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), before, "a collision: {seen:?}");
    }

    #[test]
    fn identifiers_are_the_shape_xcode_writes_and_do_not_move() {
        for role in ROLES {
            let id = oid("app", role);
            assert_eq!(id.len(), 24, "{role}: {id}");
            assert!(
                id.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_lowercase()),
                "{role}: {id}"
            );
        }
        // Regenerating a project must not rewrite every line of it.
        assert_eq!(oid("app", "target"), oid("app", "target"));
        assert_ne!(oid("app", "target"), oid("other", "target"));
    }

    #[test]
    fn the_scheme_names_the_target_the_project_defines() {
        // `xcodebuild -scheme` resolves through this identifier, and a scheme
        // pointing at a blueprint the project does not contain fails with
        // "scheme not found" — which reads as a missing file rather than a
        // mismatched one.
        let files = files("demo");
        let scheme = find(
            &files,
            "ios/demo.xcodeproj/xcshareddata/xcschemes/demo.xcscheme",
        );
        let project = find(&files, "ios/demo.xcodeproj/project.pbxproj");
        let target = oid("demo", "target");
        assert!(scheme.contains(&format!(r#"BlueprintIdentifier = "{target}""#)));
        assert!(project.contains(&format!("{target} /* demo */ = {{")));
    }

    #[test]
    fn the_gradle_source_set_points_at_what_cargo_ndk_writes() {
        // `export::android` runs `cargo ndk -o target/android/jniLibs`, and
        // this file is two levels below the project root. If these two ever
        // disagree the APK builds with no library in it and fails at launch.
        let files = files("demo");
        let gradle = find(&files, "android/app/build.gradle");
        assert!(
            gradle.contains(r#"jniLibs.srcDirs = ["../../target/android/jniLibs"]"#),
            "{gradle}"
        );
        assert!(gradle.contains(r#"abiFilters "arm64-v8a""#), "{gradle}");
    }

    #[test]
    fn the_placeholder_identifier_is_one_that_cannot_be_published() {
        // Reserved by Google and refused by the Play Store, which is the point:
        // a generated identifier should be impossible to ship by accident.
        assert_eq!(bundle_id("my-app"), "com.example.my_app");
        for (_, text) in files("my-app") {
            assert!(!text.contains("com.vieww."), "no real-looking id anywhere");
        }
    }

    #[test]
    fn the_ios_project_links_a_different_target_per_sdk() {
        // A simulator build linking the device's `.a` fails with a message
        // about architectures that never mentions the target triple.
        let files = files("demo");
        let project = find(&files, "ios/demo.xcodeproj/project.pbxproj");
        assert!(project.contains(r#""RUST_TARGET[sdk=iphoneos*]" = "aarch64-apple-ios";"#));
        assert!(
            project.contains(r#""RUST_TARGET[sdk=iphonesimulator*]" = "aarch64-apple-ios-sim";"#)
        );
        assert!(project.contains("$(PROJECT_DIR)/../target/$(RUST_TARGET)/release"));
    }

    fn find<'a>(files: &'a [(PathBuf, String)], path: &str) -> &'a str {
        files
            .iter()
            .find(|(candidate, _)| candidate == &PathBuf::from(path))
            .map(|(_, text)| text.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "no {path} in {:?}",
                    files.iter().map(|(p, _)| p).collect::<Vec<_>>()
                )
            })
    }
}
