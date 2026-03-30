use assert_cmd::cargo::cargo_bin_cmd;
use predicates::str::contains;

#[test]
fn root_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("واجهة أدوات المطور Greentic"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn secrets_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["secrets", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("أغلفة ميسّرة للأسرار"))
        .stdout(contains("فوض إلى greentic-secrets لتهيئة الأسرار لحزمة"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn config_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["config", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("إدارة إعدادات greentic-dev"))
        .stdout(contains("عيّن مفتاحًا في إعداد greentic-dev"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn tools_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["tools", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("تثبيت / تحديث أدوات Greentic المفوّضة"))
        .stdout(contains("تثبيت الأدوات المفوّضة"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn coverage_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["coverage", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("شغّل فحوصات التغطية مقابل coverage-policy.json"))
        .stdout(contains(
            "أعد استخدام تقرير target/coverage/coverage.json موجود",
        ))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn wizard_apply_help_uses_requested_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["wizard", "apply", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("طبّق AnswerDocument الخاصة بالمشغّل دون تفاعل"))
        .stdout(contains("ملف الإجابات"))
        .stdout(contains("تخطَّ مطالبة التأكيد التفاعلية"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn wizard_help_uses_requested_locale_for_answers_flag() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.args(["wizard", "--help", "--locale", "ar"]);
    cmd.assert()
        .success()
        .stdout(contains("ملف الإجابات"))
        .stdout(contains("وضع الواجهة الأمامية"))
        .stdout(contains("اعرض المساعدة"));
}

#[test]
fn secrets_runtime_error_uses_env_locale() {
    let mut cmd = cargo_bin_cmd!("greentic-dev");
    cmd.env("LC_ALL", "ar")
        .env("GREENTIC_DEV_BIN_GREENTIC_SECRETS", "/tmp/does-not-exist")
        .args(["secrets", "init", "--pack", "dummy.gtpack"]);
    cmd.assert().failure().stderr(contains(
        "يشير GREENTIC_DEV_BIN_GREENTIC_SECRETS إلى ملف تنفيذي غير موجود",
    ));
}
