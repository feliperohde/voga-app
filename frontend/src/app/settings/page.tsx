'use client';

import { useEffect, useState } from 'react';
import { PageHeader } from '@/components/PageHeader';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { AiSettings } from '@/components/AiSettings';
import { AppearanceSettings } from '@/components/AppearanceSettings';
import { CollectSettings } from '@/components/CollectSettings';
import { FlowsintSettings } from '@/components/FlowsintSettings';
import { About } from '@/components/About';
import { useI18n } from '@/i18n';

export default function SettingsPage() {
  const { t } = useI18n();
  const [tab, setTab] = useState('ai');
  // /settings?tab=flowsint opens that tab (links from other pages)
  useEffect(() => {
    const wanted = new URLSearchParams(window.location.search).get('tab');
    if (wanted) setTab(wanted);
  }, []);
  return (
    <div>
      <PageHeader title={t('nav.settings')} />
      <div className="mx-auto max-w-3xl px-4 py-6 sm:px-8">
        <Tabs value={tab} onValueChange={setTab}>
          <TabsList>
            <TabsTrigger value="ai">{t('settings.tabs.ai')}</TabsTrigger>
            <TabsTrigger value="collect">{t('settings.tabs.collect')}</TabsTrigger>
            <TabsTrigger value="flowsint">flowsint</TabsTrigger>
            <TabsTrigger value="appearance">{t('settings.tabs.appearance')}</TabsTrigger>
            <TabsTrigger value="about">{t('settings.tabs.about')}</TabsTrigger>
          </TabsList>
          <TabsContent value="ai" className="pt-4">
            <AiSettings />
          </TabsContent>
          <TabsContent value="collect" className="pt-4">
            <CollectSettings />
          </TabsContent>
          <TabsContent value="flowsint" className="pt-4">
            <FlowsintSettings />
          </TabsContent>
          <TabsContent value="appearance" className="pt-4">
            <AppearanceSettings />
          </TabsContent>
          <TabsContent value="about" className="pt-4">
            <About />
          </TabsContent>
        </Tabs>
      </div>
    </div>
  );
}
